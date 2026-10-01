#[test]
fn fs_async_file_operations_observe_abort_before_io_and_between_chunks() {
    // Unrun regression: pre-aborted and pre-dispatch signals cannot mutate
    // files, while an iterable write stops before its next chunk.
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_async_abort");
    fs::write(dir.join("index.js"), r#"var fs = require('node:fs'), promises = require('node:fs/promises');
        module.exports = async function(root) {
            var path = root + '/value.txt';
            fs.writeFileSync(path, 'keep');
            var pre = new AbortController(); pre.abort('pre');
            function details(error) { return [error.name, error.code, error.cause]; }
            var promiseRead, promiseWrite, promiseAppend;
            try { await promises.readFile(path, { signal: pre.signal }); } catch (error) { promiseRead = details(error); }
            try { await promises.writeFile(path, 'changed', { signal: pre.signal }); } catch (error) { promiseWrite = details(error); }
            try { await promises.appendFile(path, 'changed', { signal: pre.signal }); } catch (error) { promiseAppend = details(error); }
            var callbackRead = await new Promise(function(resolve) { fs.readFile(path, { signal: pre.signal }, function(error) { resolve(details(error)); }); });
            var callbackWrite = await new Promise(function(resolve) { fs.writeFile(path, 'changed', { signal: pre.signal }, function(error) { resolve(details(error)); }); });
            var callbackAppend = await new Promise(function(resolve) { fs.appendFile(path, 'changed', { signal: pre.signal }, function(error) { resolve(details(error)); }); });
            var beforeIo = fs.readFileSync(path, 'utf8');
            var during = new AbortController(), pending = promises.writeFile(path, 'changed', { signal: during.signal }), preDispatch;
            during.abort('dispatch');
            try { await pending; } catch (error) { preDispatch = details(error); }
            var invalidPromise, invalidCallback, promiseGetterCall, promiseGetter, callbackGetter;
            try { await promises.writeFile(path, 'changed', { signal: 7 }); } catch (error) { invalidPromise = error.code; }
            invalidCallback = await new Promise(function(resolve) { fs.writeFile(path, 'changed', { signal: 7 }, function(error) { resolve(error.code); }); });
            try { var getterPending = promises.readFile(path, { get signal() { throw new Error('signal getter'); } }); promiseGetterCall = getterPending instanceof Promise; await getterPending; } catch (error) { promiseGetter = error.message; }
            callbackGetter = await new Promise(function(resolve) { fs.writeFile(path, 'changed', { get signal() { throw new Error('signal getter'); } }, function(error) { resolve(error.message); }); });
            var scalarSignal = new AbortController(), scalarError, iterableSignal = new AbortController(), iterableError, readSignal = new AbortController(), readError;
            try { await promises.writeFile(path, 'changed', { signal: scalarSignal.signal, get encoding() { scalarSignal.abort('scalar'); return 'utf8'; } }); } catch (error) { scalarError = details(error); }
            try { await promises.writeFile(path, ['changed'], { signal: iterableSignal.signal, get flag() { iterableSignal.abort('iterable'); return 'w'; } }); } catch (error) { iterableError = details(error); }
            try { await promises.readFile(path, { signal: readSignal.signal, get encoding() { readSignal.abort('read'); return 'utf8'; } }); } catch (error) { readError = details(error); }
            var afterGetters = fs.readFileSync(path, 'utf8');
            var handle = await promises.open(path, 'r+'), handleRead, handleWrite, handleAppend, handleGetterError, handleGetterCall;
            try { await handle.readFile({ signal: pre.signal }); } catch (error) { handleRead = details(error); }
            try { await handle.writeFile('changed', { signal: pre.signal }); } catch (error) { handleWrite = details(error); }
            try { await handle.appendFile('changed', { signal: pre.signal }); } catch (error) { handleAppend = details(error); }
            var borrowedSignal = new AbortController();
            try { var borrowedPending = handle.writeFile('changed', { signal: borrowedSignal.signal, get encoding() { borrowedSignal.abort('borrowed'); return 'utf8'; } }); handleGetterCall = borrowedPending instanceof Promise; await borrowedPending; } catch (error) { handleGetterError = details(error); }
            var descriptor = handle.fd, fdSignal = new AbortController(), fdGetterError;
            Object.defineProperty(handle, 'fd', { configurable: true, get: function() { fdSignal.abort('descriptor'); return descriptor; } });
            try { await handle.writeFile('changed', { signal: fdSignal.signal }); } catch (error) { fdGetterError = details(error); }
            var coercionErrors = [];
            for (var operation of ['read', 'scalar', 'iterable']) {
                var coercionSignal = new AbortController(), label = operation;
                Object.defineProperty(handle, 'fd', { configurable: true, value: operation === 'iterable'
                    ? { valueOf: function() { return this; }, toString: function() { coercionSignal.abort(label); return String(descriptor); } }
                    : { valueOf: function() { coercionSignal.abort(label); return descriptor; } } });
                try {
                    if (operation === 'read') await handle.readFile({ signal: coercionSignal.signal });
                    else if (operation === 'scalar') await handle.writeFile('changed', { signal: coercionSignal.signal });
                    else await handle.writeFile(['changed'], { signal: coercionSignal.signal });
                    coercionErrors.push(null);
                } catch (error) { coercionErrors.push(details(error)); }
            }
            Object.defineProperty(handle, 'fd', { configurable: true, writable: true, value: descriptor });
            var borrowedIntact = await handle.readFile('utf8');
            await handle.close();
            var handleIntact = fs.readFileSync(path, 'utf8');
            var between = new AbortController(), partial, betweenError;
            async function* chunks() { yield 'A'; between.abort('between'); yield 'B'; }
            try { await promises.writeFile(path, chunks(), { signal: between.signal }); } catch (error) { betweenError = details(error); }
            partial = fs.readFileSync(path, 'utf8');
            return [promiseRead, promiseWrite, promiseAppend, callbackRead, callbackWrite, callbackAppend, beforeIo, preDispatch, invalidPromise, invalidCallback, promiseGetterCall, promiseGetter, callbackGetter, scalarError, iterableError, readError, afterGetters, handleRead, handleWrite, handleAppend, handleGetterCall, handleGetterError, fdGetterError, coercionErrors, borrowedIntact, handleIntact, betweenError, partial];
        };"#).unwrap();
    let empty_node_modules = temp_registry("builtin_fs_async_abort_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseAsyncAbort = module.exports;");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let arguments = CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(CString::new("exerciseAsyncAbort").unwrap().as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[["AbortError","ABORT_ERR","pre"],["AbortError","ABORT_ERR","pre"],["AbortError","ABORT_ERR","pre"],["AbortError","ABORT_ERR","pre"],["AbortError","ABORT_ERR","pre"],["AbortError","ABORT_ERR","pre"],"keep",["AbortError","ABORT_ERR","dispatch"],"ERR_INVALID_ARG_TYPE","ERR_INVALID_ARG_TYPE",true,"signal getter","signal getter",["AbortError","ABORT_ERR","scalar"],["AbortError","ABORT_ERR","iterable"],["AbortError","ABORT_ERR","read"],"keep",["AbortError","ABORT_ERR","pre"],["AbortError","ABORT_ERR","pre"],["AbortError","ABORT_ERR","pre"],true,["AbortError","ABORT_ERR","borrowed"],["AbortError","ABORT_ERR","descriptor"],[["AbortError","ABORT_ERR","read"],["AbortError","ABORT_ERR","scalar"],["AbortError","ABORT_ERR","iterable"]],"keep","keep",["AbortError","ABORT_ERR","between"],"A"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_promise_write_file_consumes_iterables_and_keeps_descriptor_ownership() {
    // Unrun regression: iterable chunks must be written in order through one
    // descriptor; an iterator failure rejects and closes only an opened path.
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_write_iterable");
    fs::write(
        dir.join("index.js"),
        r#"var fs = require('node:fs'), promises = require('node:fs/promises'), Readable = require('node:stream').Readable;
        module.exports = async function(root) {
            var path = root + '/iterable.txt';
            function* first() { yield 'a'; yield Buffer.from('62', 'hex'); yield new Uint8Array([99]); }
            await promises.writeFile(path, first());
            async function* second() { yield 'd'; await Promise.resolve(); yield Buffer.from('e'); }
            await promises.appendFile(path, second());
            await promises.appendFile(path, Readable.from(['f', 'g']));
            var combined = fs.readFileSync(path, 'utf8');
            var encodingReads = 0, encodingPath = root + '/encoded.txt';
            await promises.writeFile(encodingPath, ['41', '42'], { get encoding() { encodingReads++; return 'hex'; } });
            var encodedText = fs.readFileSync(encodingPath, 'utf8');
            var handle = await promises.open(path, 'r+');
            await handle.writeFile((function*() { yield 'X'; yield 'Y'; })());
            var position = await handle.write('Z');
            await handle.close();
            var afterHandle = fs.readFileSync(path, 'utf8');
            var appendHandle = await promises.open(path, 'a');
            await appendHandle.appendFile((async function*() { yield '!'; })());
            await appendHandle.close();
            var afterAppend = fs.readFileSync(path, 'utf8');
            var badEmpty, badData;
            try { await promises.writeFile(path, [], { encoding: 'invalid-encoding' }); }
            catch (error) { badEmpty = error instanceof TypeError; }
            try { await promises.writeFile(path, ['x'], { encoding: 'invalid-encoding' }); }
            catch (error) { badData = error instanceof TypeError; }
            var untouched = fs.readFileSync(path, 'utf8') === afterAppend;
            var failed;
            try { await promises.writeFile(path, (async function*() { yield 'q'; throw new Error('iterator failed'); })()); }
            catch (error) { failed = error.message; }
            await promises.writeFile(path, []);
            return [combined, encodedText, encodingReads, position.bytesWritten, afterHandle, afterAppend, badEmpty, badData, untouched, failed, fs.readFileSync(path).length];
        };"#,
    ).unwrap();
    let empty_node_modules = temp_registry("builtin_fs_write_iterable_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWriteIterable = module.exports;");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let arguments = CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseWriteIterable").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["abcdefg","AB",1,1,"XYZdefg","XYZdefg!",true,true,true,"iterator failed",0]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_sync_and_promise_apis_operate_on_the_host_filesystem() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_promises");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); var promises = require('node:fs/promises'); module.exports = async function (root) { var work = root + '/work', original = work + '/value.txt', renamed = work + '/renamed.txt'; fs.mkdirSync(work, { recursive: true }); await new Promise(function(resolve, reject) { fs.writeFile(original, 'one', function(error) { error ? reject(error) : resolve(); }); }); await new Promise(function(resolve, reject) { fs.appendFile(original, Buffer.from('two'), function(error) { error ? reject(error) : resolve(); }); }); var text = await promises.readFile(original, 'utf8'), callbackText = await new Promise(function(resolve, reject) { fs.readFile(original, 'utf8', function(error, value) { error ? reject(error) : resolve(value); }); }), stat = await new Promise(function(resolve, reject) { fs.stat(original, function(error, value) { error ? reject(error) : resolve(value); }); }), entries = fs.readdirSync(work, { withFileTypes: true }); await promises.rename(original, renamed); var renamedExists = fs.existsSync(renamed), missingCode; try { await promises.readFile(work + '/missing'); } catch (error) { missingCode = [error.code, error.path, error.syscall]; } await promises.unlink(renamed); await promises.rm(work, { recursive: true }); return [text, callbackText, stat.isFile(), stat.isDirectory(), stat.size, entries[0].name, entries[0].isFile(), renamedExists, fs.existsSync(renamed), fs.existsSync(work), missingCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_promises_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsPromises = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseFsPromises").unwrap();
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        format!(
            r#"["onetwo","onetwo",true,false,6,"value.txt",true,true,false,false,["ENOENT","{}/work/missing","open"]]"#,
            dir.to_string_lossy()
        )
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[cfg(unix)]
#[test]
fn fs_cp_finalizes_new_directory_modes_after_children_and_errors() {
    // Unrun regression: read-only source directories remain readable while
    // children are copied, then new destinations acquire the source mode.
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_cp_directory_modes");
    fs::write(dir.join("index.js"), r#"var fs = require('node:fs'), promises = require('node:fs/promises');
        module.exports = async function(root) {
            var source = root + '/source', nested = source + '/nested';
            fs.mkdirSync(nested, { recursive: true });
            fs.writeFileSync(nested + '/value.txt', 'copied');
            fs.chmodSync(nested, 365); fs.chmodSync(source, 488);
            fs.cpSync(source, root + '/sync', { recursive: true });
            await promises.cp(source, root + '/promised', { recursive: true });
            await new Promise(function(resolve, reject) { fs.cp(source, root + '/callback', { recursive: true }, function(error) { error ? reject(error) : resolve(); }); });
            fs.mkdirSync(root + '/existing/nested', { recursive: true });
            fs.chmodSync(root + '/existing/nested', 448);
            fs.chmodSync(root + '/existing', 448);
            fs.cpSync(source, root + '/existing', { recursive: true });
            var fail = function(src) { if (src.endsWith('/value.txt')) throw new Error('child failure'); return true; };
            var syncError, asyncError, callbackError;
            try { fs.cpSync(source, root + '/sync-fail', { recursive: true, filter: fail }); } catch (error) { syncError = error.message; }
            try { await promises.cp(source, root + '/async-fail', { recursive: true, filter: function(src) { return Promise.resolve().then(function() { return fail(src); }); } }); } catch (error) { asyncError = error.message; }
            await new Promise(function(resolve) { fs.cp(source, root + '/callback-fail', { recursive: true, filter: function(src) { return Promise.resolve().then(function() { return fail(src); }); } }, function(error) { callbackError = error && error.message; resolve(); }); });
            var mode = function(path) { return fs.statSync(path).mode & 4095; };
            var paths = ['sync', 'promised', 'callback', 'existing', 'sync-fail', 'async-fail', 'callback-fail'];
            var modes = paths.map(function(name) { var path = root + '/' + name; return [mode(path), mode(path + '/nested')]; });
            var content = fs.readFileSync(root + '/sync/nested/value.txt', 'utf8');
            [source].concat(paths.map(function(name) { return root + '/' + name; })).forEach(function(path) { fs.chmodSync(path + '/nested', 448); fs.chmodSync(path, 448); });
            return [modes, content, syncError, asyncError, callbackError];
        };"#).unwrap();
    let modules = temp_registry("builtin_fs_cp_directory_modes_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &dir, "index.js").unwrap();
    let source = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseCpDirectoryModes = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseCpDirectoryModes").unwrap();
    let arguments = CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap()).unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let parsed: serde_json::Value = serde_json::from_str(&unsafe { CStr::from_ptr(result) }.to_string_lossy()).unwrap();
    assert_eq!(parsed[0], serde_json::json!([[488,365],[488,365],[488,365],[448,448],[488,365],[488,365],[488,365]]));
    assert_eq!(parsed[1], "copied");
    assert_eq!(parsed[2], "child failure");
    assert_eq!(parsed[3], "child failure");
    assert_eq!(parsed[4], "child failure");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(modules);
}

#[test]
fn fs_copy_realpath_and_mkdtemp_work_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_paths");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source.txt', copied = root + '/copied.txt', prefix = root + '/temporary-'; fs.writeFileSync(source, 'copy-value'); fs.copyFileSync(source, copied); var copiedValue = fs.readFileSync(copied, 'utf8'), resolved = await promises.realpath(copied); var temporary = await new Promise(function(resolve, reject) { fs.mkdtemp(prefix, function(error, path) { error ? reject(error) : resolve(path); }); }); var callbackPath = await new Promise(function(resolve, reject) { fs.realpath.native(source, function(error, path) { error ? reject(error) : resolve(path); }); }); await promises.unlink(source); await promises.unlink(copied); await promises.rmdir(temporary); return [copiedValue, resolved.slice(-10), callbackPath.slice(-10), temporary.indexOf(prefix) === 0, fs.existsSync(temporary), typeof fs.realpathSync.native]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_paths_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsPaths = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsPaths").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["copy-value","copied.txt","source.txt",true,false,"function"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_disposable_temporary_directories_remove_nested_contents_once() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_disposable_mkdtemp");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var sync = fs.mkdtempDisposableSync(root + '/sync-'); fs.mkdirSync(sync.path + '/nested'); fs.writeFileSync(sync.path + '/nested/value.txt', 'sync'); var syncPath = sync.path, syncSymbol = Symbol.dispose ? sync[Symbol.dispose] === sync.remove : true; sync.remove(); sync.remove(); var disposable = await promises.mkdtempDisposable(root + '/async-', { encoding: 'buffer' }), asyncPath = disposable.path.toString(), asyncSymbol = Symbol.asyncDispose ? disposable[Symbol.asyncDispose] === disposable.remove : true; fs.mkdirSync(asyncPath + '/nested'); fs.writeFileSync(asyncPath + '/nested/value.txt', 'async'); var first = disposable.remove(), sameRemoval = first === disposable.remove(); await first; await disposable.remove(); return [syncPath.indexOf(root + '/sync-') === 0, fs.existsSync(syncPath), syncSymbol, Buffer.isBuffer(disposable.path), asyncPath.indexOf(root + '/async-') === 0, fs.existsSync(asyncPath), asyncSymbol, sameRemoval]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_disposable_mkdtemp_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDisposableMkdtemp = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseDisposableMkdtemp").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,false,true,true,true,false,true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_open_as_blob_exposes_blob_reads_and_detects_file_changes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_open_as_blob");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (root) { var path = root + '/blob.txt'; fs.writeFileSync(path, 'abcdef'); var blob = await fs.openAsBlob(path, { type: 'Text/Plain' }), slice = blob.slice(1, 4, 'APPLICATION/X'), bytes = Array.from(await blob.bytes()), buffer = Array.from(new Uint8Array(await blob.arrayBuffer())), reader = blob.stream().getReader(), streamed = await reader.read(); reader.releaseLock(); var missingCode; try { await fs.openAsBlob(root + '/missing'); } catch (error) { missingCode = error.code; } fs.writeFileSync(path, 'changed-value'); var changedName; try { await blob.text(); } catch (error) { changedName = error.name; } var sliceChangedName; try { await slice.text(); } catch (error) { sliceChangedName = error.name; } return [blob instanceof Blob, blob.size, blob.type, Buffer.from(bytes).toString(), buffer.length, Buffer.from(streamed.value).toString(), streamed.done, slice.size, slice.type, missingCode, changedName, sliceChangedName]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_open_as_blob_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseOpenAsBlob = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseOpenAsBlob").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,6,"text/plain","abcdef",6,"abcdef",false,3,"application/x","ENOENT","NotReadableError","NotReadableError"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_cp_recursively_copies_directories_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_cp");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source', sync = root + '/sync', asyncPath = root + '/async', callbackPath = root + '/callback'; fs.mkdirSync(source + '/nested', { recursive: true }); fs.writeFileSync(source + '/nested/value.txt', 'copied'); fs.cpSync(source, sync, { recursive: true }); await promises.cp(source, asyncPath, { recursive: true }); await new Promise(function(resolve, reject) { fs.cp(source, callbackPath, { recursive: true }, function(error) { error ? reject(error) : resolve(); }); }); fs.writeFileSync(sync + '/nested/value.txt', 'kept'); fs.cpSync(source, sync, { recursive: true, force: false }); var existingCode; try { fs.cpSync(source, sync, { recursive: true, force: false, errorOnExist: true }); } catch (error) { existingCode = error.code; } var empty = root + '/empty', targets = ['sync-empty', 'promise-empty', 'callback-empty']; fs.mkdirSync(empty); targets.forEach(function(name) { fs.mkdirSync(root + '/' + name); }); var emptyCodes = []; try { fs.cpSync(empty, root + '/' + targets[0], { recursive: true, force: false, errorOnExist: true }); } catch (error) { emptyCodes.push(error.code); } try { await promises.cp(empty, root + '/' + targets[1], { recursive: true, force: false, errorOnExist: true }); } catch (error) { emptyCodes.push(error.code); } await new Promise(function(resolve) { fs.cp(empty, root + '/' + targets[2], { recursive: true, force: false, errorOnExist: true }, function(error) { emptyCodes.push(error && error.code); resolve(); }); }); return [emptyCodes, fs.readFileSync(sync + '/nested/value.txt', 'utf8'), fs.readFileSync(asyncPath + '/nested/value.txt', 'utf8'), fs.readFileSync(callbackPath + '/nested/value.txt', 'utf8'), existingCode]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_cp_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsCp = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsCp").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[["EEXIST","EEXIST","EEXIST"],"kept","copied","copied","EEXIST"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_cp_honors_filters_links_and_timestamp_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_cp_options");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source', sync = root + '/sync', promised = root + '/promised', callback = root + '/callback'; fs.mkdirSync(source + '/nested', { recursive: true }); fs.writeFileSync(source + '/keep.txt', 'keep'); fs.writeFileSync(source + '/skip.txt', 'skip'); fs.writeFileSync(source + '/nested/value.txt', 'nested'); fs.symlinkSync('keep.txt', source + '/link'); fs.utimesSync(source + '/keep.txt', new Date(1000000), new Date(2000000)); var syncSeen = []; fs.cpSync(source, sync, { recursive: true, verbatimSymlinks: true, preserveTimestamps: true, filter: function(src) { syncSeen.push(src); return !src.endsWith('/skip.txt'); } }); var asyncSeen = []; await promises.cp(source, promised, { recursive: true, dereference: true, filter: async function(src) { asyncSeen.push(src); await Promise.resolve(); return !src.endsWith('/nested'); } }); var callbackSeen = []; await new Promise(function(resolve, reject) { fs.cp(source, callback, { recursive: true, filter: function(src) { callbackSeen.push(src); return Promise.resolve(!src.endsWith('/skip.txt')); } }, function(error) { error ? reject(error) : resolve(); }); }); var directoryCode; try { fs.cpSync(source, root + '/not-recursive'); } catch (error) { directoryCode = error.code; } var asyncFilterError; try { fs.cpSync(source + '/keep.txt', root + '/invalid-filter', { filter: function() { return Promise.resolve(true); } }); } catch (error) { asyncFilterError = error.name; } fs.symlinkSync(source + '/keep.txt', root + '/alias'); var sameCode, asyncSameCode; try { fs.cpSync(root + '/alias', source + '/keep.txt', { dereference: true }); } catch (error) { sameCode = error.code; } try { await promises.cp(root + '/alias', source + '/keep.txt', { dereference: true }); } catch (error) { asyncSameCode = error.code; } return [fs.existsSync(sync + '/keep.txt'), fs.existsSync(sync + '/skip.txt'), fs.readlinkSync(sync + '/link'), Math.abs(fs.statSync(sync + '/keep.txt').mtimeMs - 2000000) < 2, syncSeen.length, fs.readFileSync(promised + '/link', 'utf8'), fs.existsSync(promised + '/nested'), asyncSeen.length, fs.existsSync(callback + '/keep.txt'), fs.existsSync(callback + '/skip.txt'), callbackSeen.length, directoryCode, asyncFilterError, sameCode, asyncSameCode, fs.readFileSync(source + '/keep.txt', 'utf8')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_cp_options_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsCpOptions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsCpOptions").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,false,"keep.txt",true,6,"keep",false,5,true,false,6,"ERR_FS_EISDIR","TypeError","EEXIST","EEXIST","keep"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_promise_file_handles_manage_repeated_operations_and_streams() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_file_handle");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'), stream = require('node:stream'); module.exports = async function (path) { var handle = await promises.open(path, 'w+'), originalFd = handle.fd; await handle.writeFile('abcdef'); await handle.appendFile('ghi'); var before = await handle.stat(); await handle.truncate(5); var value = await handle.readFile('utf8'), reader = handle.createReadStream({ highWaterMark: 2 }), chunks = []; reader.on('data', function(chunk) { chunks.push(chunk.toString()); }); await new Promise(function(resolve, reject) { reader.on('error', reject); reader.on('end', resolve); }); await handle.close(); var closedCode; try { await handle.readFile(); } catch (error) { closedCode = error.code; } return [originalFd >= 10, before.size, value, chunks, reader instanceof fs.ReadStream, reader instanceof stream.Readable, handle.fd, handle.closed, closedCode, Symbol.asyncDispose ? typeof handle[Symbol.asyncDispose] : 'unavailable']; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_file_handle_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsFileHandle = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("handle.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsFileHandle").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,9,"",[],true,true,-1,true,"EBADF","function"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_file_handles_support_positioned_buffer_reads_and_writes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_file_handle_positions");
    fs::write(
            dir.join("index.js"),
            "var promises = require('node:fs/promises'); module.exports = async function (path) { var handle = await promises.open(path, 'w+'); await handle.writeFile('abcdef'); var first = Buffer.alloc(4, 46), read = await handle.read(first, 1, 2, 2); var written = await handle.write(Buffer.from('XYZ'), 1, 2, 4); var textWrite = await handle.write('!', 1, 'utf8'); var sequential = Buffer.alloc(3), sequentialRead = await handle.read(sequential, 0, 3, null); var value = await handle.readFile('utf8'); await handle.close(); return [read.bytesRead, read.buffer.toString(), written.bytesWritten, written.buffer.toString(), textWrite.bytesWritten, textWrite.buffer, sequentialRead.bytesRead, sequential.toString('hex'), value]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_file_handle_positions_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsFileHandlePositions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("positioned.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsFileHandlePositions")
            .unwrap()
            .as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[2,".cd.",2,"XYZ",1,"!",0,"000000",""]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_directory_handles_support_reads_callbacks_and_async_iteration() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_directory_handles");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var work = root + '/entries'; fs.mkdirSync(work); fs.writeFileSync(work + '/a.txt', 'a'); fs.mkdirSync(work + '/nested'); var sync = fs.opendirSync(work), first = sync.readSync(), second = sync.readSync(); sync.closeSync(); var closedCode; try { sync.readSync(); } catch (error) { closedCode = error.code; } var callbackName = await new Promise(function(resolve, reject) { fs.opendir(work, function(error, opened) { if (error) return reject(error); opened.read(function(readError, entry) { if (readError) return reject(readError); opened.close(function(closeError) { closeError ? reject(closeError) : resolve(entry.name); }); }); }); }); var opened = await promises.opendir(work), iterated = []; for await (var entry of opened) iterated.push([entry.name, entry.isFile(), entry.isDirectory()]); iterated.sort(function(left, right) { return left[0].localeCompare(right[0]); }); return [[first.name, second.name].sort(), sync instanceof fs.Dir, sync.closed, closedCode, typeof callbackName, iterated, opened.closed]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_directory_handles_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsDirectoryHandles = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsDirectoryHandles").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["a.txt","nested"],true,true,"ERR_DIR_CLOSED","string",[["a.txt",true,false],["nested",false,true]],true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_links_and_permissions_use_host_filesystem_semantics() {
    use std::ffi::{CStr, CString};
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_registry("builtin_fs_links");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source.txt', hard = root + '/hard.txt', symbolic = root + '/symbolic.txt'; fs.writeFileSync(source, 'linked'); fs.linkSync(source, hard); await promises.symlink(source, symbolic); var target = await new Promise(function(resolve, reject) { fs.readlink(symbolic, function(error, value) { error ? reject(error) : resolve(value); }); }); await promises.chmod(source, 384); return [fs.readFileSync(hard, 'utf8'), fs.readFileSync(symbolic, 'utf8'), target, fs.readlinkSync(symbolic, 'buffer').toString(), fs.existsSync(hard), fs.existsSync(symbolic)]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_links_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsLinks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsLinks").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let source_path = dir.join("source.txt").to_string_lossy().into_owned();
    assert_eq!(
        result,
        format!(r#"["linked","linked","{0}","{0}",true,true]"#, source_path)
    );
    assert_eq!(
        fs::metadata(dir.join("source.txt"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_stats_and_utimes_use_host_timestamps_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_timestamps");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { fs.writeFileSync(path, 'time'); fs.utimesSync(path, 100, 200); var sync = fs.statSync(path); await new Promise(function(resolve, reject) { fs.utimes(path, new Date(300000), new Date(400000), function(error) { error ? reject(error) : resolve(); }); }); var callback = await promises.stat(path); await promises.utimes(path, 500, 600); var handle = await promises.open(path, 'r+'); await handle.utimes(700, 800); var finalStats = await handle.stat(); await handle.close(); return [Math.round(sync.atimeMs), Math.round(sync.mtimeMs), sync.atime.getTime(), sync.mtime.getTime(), sync.ctimeMs > 0, (sync.mode & 32768) !== 0, Math.round(callback.atimeMs), Math.round(callback.mtimeMs), Math.round(finalStats.atimeMs), Math.round(finalStats.mtimeMs), finalStats.atime.getTime(), finalStats.mtime.getTime()]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_timestamps_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsTimestamps = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("timestamps.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsTimestamps").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[100000,200000,100000,200000,true,true,300000,400000,700000,800000,700000,800000]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_ownership_apis_use_host_uid_and_gid() {
    use std::ffi::{CStr, CString};
    use std::os::unix::fs::MetadataExt;
    let dir = temp_registry("builtin_fs_ownership");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root, uid, gid) { var source = root + '/owned.txt', link = root + '/owned-link'; fs.writeFileSync(source, 'owned'); fs.symlinkSync(source, link); fs.chownSync(source, uid, gid); await promises.chown(source, uid, gid); await new Promise(function(resolve, reject) { fs.lchown(link, uid, gid, function(error) { error ? reject(error) : resolve(); }); }); var handle = await promises.open(source, 'r+'); await handle.chown(uid, gid); var stats = await handle.stat(); await handle.close(); return [stats.uid, stats.gid, fs.statSync(source).uid, fs.statSync(source).gid]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_ownership_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsOwnership = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let owner = fs::metadata(&dir).unwrap();
    let uid = owner.uid();
    let gid = owner.gid();
    let arguments = CString::new(
        serde_json::to_string(&[
            serde_json::json!(dir.to_string_lossy()),
            serde_json::json!(uid),
            serde_json::json!(gid),
        ])
        .unwrap(),
    )
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsOwnership").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, format!("[{uid},{gid},{uid},{gid}]"));
    let link_metadata = fs::symlink_metadata(dir.join("owned-link")).unwrap();
    assert_eq!((link_metadata.uid(), link_metadata.gid()), (uid, gid));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_watch_file_reports_host_changes_and_stops_cleanly() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_watch_file");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (path) { fs.writeFileSync(path, 'one'); var ignoredCalls = 0, ignored = function() { ignoredCalls++; }, watcher; var changed = new Promise(function(resolve) { watcher = fs.watchFile(path, { interval: 1, persistent: false }, function(current, previous) { fs.unwatchFile(path); resolve([previous.size, current.size, current.isFile(), watcher.hasRef(), ignoredCalls]); }); fs.watchFile(path, { interval: 1 }, ignored); fs.unwatchFile(path, ignored); }); setTimeout(function() { fs.writeFileSync(path, 'changed-value'); }, 3); var result = await changed; return [result, watcher._timer, watcher._listeners.length]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_watch_file_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsWatchFile = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("watched.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsWatchFile").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[[3,13,true,false,0],null,0]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_watch_reports_directory_rename_and_change_events() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_watch");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (root) { var work = root + '/watched'; fs.mkdirSync(work); var events = [], closeCount = 0, watcher; var completed = new Promise(function(resolve) { watcher = fs.watch(work, { interval: 1, persistent: false }, function(type, name) { events.push([type, name]); if (events.length === 2) { watcher.close(); resolve(); } }); watcher.once('close', function() { closeCount++; }); }); setTimeout(function() { fs.writeFileSync(work + '/value.txt', 'a'); }, 3); setTimeout(function() { fs.writeFileSync(work + '/value.txt', 'expanded'); }, 12); await completed; var controller = new AbortController(), aborted = fs.watch(work, { interval: 1, signal: controller.signal, encoding: 'buffer' }); controller.abort(); return [watcher instanceof fs.FSWatcher, events, watcher.closed, watcher._timer, watcher.hasRef(), closeCount, aborted.closed, aborted._timer]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_watch_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsWatch = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsWatch").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,[["rename","value.txt"],["change","value.txt"]],true,null,false,1,true,null]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_glob_supports_patterns_exclusions_and_async_iteration() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_glob");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { fs.mkdirSync(root + '/src/nested', { recursive: true }); fs.writeFileSync(root + '/root.txt', 'root'); fs.writeFileSync(root + '/src/main.js', 'js'); fs.writeFileSync(root + '/src/nested/value.txt', 'txt'); fs.writeFileSync(root + '/src/nested/skip.txt', 'skip'); var sync = fs.globSync('**/*.txt', { cwd: root, exclude: '**/skip.txt' }).sort(); var callback = await new Promise(function(resolve, reject) { fs.glob(['src/*.js', 'root.?xt'], { cwd: root }, function(error, values) { error ? reject(error) : resolve(values.sort()); }); }); var asyncValues = []; for await (var value of promises.glob('src/**', { cwd: root })) asyncValues.push(value); asyncValues.sort(); var dirents = fs.globSync('src/*', { cwd: root, withFileTypes: true }); return [sync, callback, asyncValues, dirents.map(function(entry) { return [entry.name, entry.isFile(), entry.isDirectory()]; }).sort()]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_glob_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsGlob = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsGlob").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["root.txt","src/nested/value.txt"],["root.txt","src/main.js"],["src/main.js","src/nested","src/nested/skip.txt","src/nested/value.txt"],[["main.js",true,false],["nested",false,true]]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_access_checks_host_permissions_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_access");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var path = root + '/access.txt', missing = root + '/missing.txt'; fs.writeFileSync(path, 'access'); fs.accessSync(path, fs.constants.R_OK | fs.constants.W_OK); await promises.access(root, fs.constants.X_OK); var callback = await new Promise(function(resolve, reject) { fs.access(path, fs.constants.F_OK, function(error) { error ? reject(error) : resolve(true); }); }); var syncError, callbackError, promiseError; try { fs.accessSync(missing); } catch (error) { syncError = [error.code, error.path, error.syscall]; } await new Promise(function(resolve) { fs.access(missing, function(error) { callbackError = [error.code, error.path, error.syscall]; resolve(); }); }); try { await promises.access(missing, fs.constants.R_OK); } catch (error) { promiseError = [error.code, error.path, error.syscall]; } return [callback, syncError, callbackError, promiseError]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_access_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsAccess = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsAccess").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let missing = dir.join("missing.txt").to_string_lossy().into_owned();
    assert_eq!(
        result,
        format!(
            r#"[true,["ENOENT","{0}","access"],["ENOENT","{0}","access"],["ENOENT","{0}","access"]]"#,
            missing
        )
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_numeric_descriptors_support_sync_and_callback_io() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_numeric_descriptors");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (path) { var fd = fs.openSync(path, 'w+'), source = Buffer.from('abcdef'), firstWrite = fs.writeSync(fd, source, 1, 3, 0), secondWrite = fs.writeSync(fd, '!', 3, 'utf8'); fs.fsyncSync(fd); fs.fdatasyncSync(fd); var buffer = Buffer.alloc(6, 46), firstRead = fs.readSync(fd, buffer, 1, 4, 0); fs.ftruncateSync(fd, 3); var size = fs.fstatSync(fd).size; fs.closeSync(fd); var closedCode; try { fs.fstatSync(fd); } catch (error) { closedCode = error.code; } var callbackResult = await new Promise(function(resolve, reject) { fs.open(path, 'r+', function(openError, callbackFd) { if (openError) return reject(openError); fs.write(callbackFd, Buffer.from('XYZ'), 0, 3, 0, function(writeError, written, original) { if (writeError) return reject(writeError); var target = Buffer.alloc(3); fs.read(callbackFd, target, 0, 3, 0, function(readError, read, returned) { if (readError) return reject(readError); fs.fstat(callbackFd, function(statError, stats) { if (statError) return reject(statError); fs.close(callbackFd, function(closeError) { closeError ? reject(closeError) : resolve([written, original.toString(), read, returned === target, target.toString(), stats.size]); }); }); }); }); }); }); return [firstWrite, secondWrite, firstRead, buffer.toString(), size, closedCode, callbackResult, fs.readFileSync(path, 'utf8')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_numeric_descriptors_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsNumericDescriptors = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("descriptor.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsNumericDescriptors")
            .unwrap()
            .as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[3,1,4,".bcd!.",3,"EBADF",[3,"XYZ",3,true,"XYZ",3],"XYZ"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_scatter_gather_io_works_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_scatter_gather");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { var fd = fs.openSync(path, 'w+'), written = fs.writevSync(fd, [Buffer.from('ab'), Buffer.from('cd')], 0), first = Buffer.alloc(2), second = Buffer.alloc(3), read = fs.readvSync(fd, [first, second], 0); var callback = await new Promise(function(resolve, reject) { var output = [Buffer.from('e'), Buffer.from('f')]; fs.writev(fd, output, 4, function(writeError, bytesWritten, returnedWrite) { if (writeError) return reject(writeError); var input = [Buffer.alloc(3), Buffer.alloc(3)]; fs.readv(fd, input, 0, function(readError, bytesRead, returnedRead) { readError ? reject(readError) : resolve([bytesWritten, returnedWrite === output, bytesRead, returnedRead === input, input[0].toString(), input[1].toString()]); }); }); }); fs.closeSync(fd); var handle = await promises.open(path, 'r+'), handleWriteBuffers = [Buffer.from('X'), Buffer.from('Y')], handleWrite = await handle.writev(handleWriteBuffers, 1), handleReadBuffers = [Buffer.alloc(3), Buffer.alloc(3)], handleRead = await handle.readv(handleReadBuffers, 0); await handle.close(); return [written, read, first.toString(), second.subarray(0, 2).toString(), callback, handleWrite.bytesWritten, handleWrite.buffers === handleWriteBuffers, handleRead.bytesRead, handleRead.buffers === handleReadBuffers, handleReadBuffers[0].toString(), handleReadBuffers[1].toString(), fs.readFileSync(path, 'utf8')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_scatter_gather_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsScatterGather = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("vectors.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsScatterGather").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[4,4,"ab","cd",[2,true,6,true,"abc","def"],2,true,6,true,"aXY","def","aXYdef"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_statfs_reports_host_capacity_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_statfs");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { var sync = fs.statfsSync(path), bigint = fs.statfsSync(path, { bigint: true }); var callback = await new Promise(function(resolve, reject) { fs.statfs(path, function(error, value) { error ? reject(error) : resolve(value); }); }); var promised = await promises.statfs(path), handle = await promises.open(path + '/value.txt', 'w+'), handled = await handle.statfs({ bigint: true }); await handle.close(); return [sync instanceof fs.StatFs, sync.bsize > 0, sync.blocks > 0, sync.bfree >= sync.bavail, sync.files >= sync.ffree, typeof bigint.bsize, bigint.bsize.toString() === String(sync.bsize), callback.bsize, promised.bsize, typeof handled.blocks, handled.blocks.toString() === String(sync.blocks)]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_statfs_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsStatfs = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsStatfs").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let values = parsed.as_array().unwrap();
    assert_eq!(
        &values[..7],
        &[
            serde_json::json!(true),
            serde_json::json!(true),
            serde_json::json!(true),
            serde_json::json!(true),
            serde_json::json!(true),
            serde_json::json!("bigint"),
            serde_json::json!(true)
        ]
    );
    assert_eq!(values[7], values[8]);
    assert_eq!(
        &values[9..],
        &[serde_json::json!("bigint"), serde_json::json!(true)]
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_lstat_and_dirents_identify_symbolic_links() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_lstat_symlinks");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var target = root + '/target.txt', link = root + '/target-link'; fs.writeFileSync(target, 'data'); fs.symlinkSync(target, link); var followed = fs.statSync(link), own = fs.lstatSync(link), callback = await new Promise(function(resolve, reject) { fs.lstat(link, function(error, value) { error ? reject(error) : resolve(value); }); }), promised = await promises.lstat(link), entry = fs.readdirSync(root, { withFileTypes: true }).filter(function(value) { return value.name === 'target-link'; })[0]; return [followed.isFile(), followed.isSymbolicLink(), own.isFile(), own.isSymbolicLink(), (own.mode & fs.constants.S_IFMT) === fs.constants.S_IFLNK, own.size === target.length, callback.isSymbolicLink(), promised.isSymbolicLink(), entry.isFile(), entry.isSymbolicLink()]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_lstat_symlinks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsLstatSymlinks = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsLstatSymlinks").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,false,false,true,true,true,true,true,false,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_lutimes_updates_links_without_touching_targets() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_lutimes");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var target = root + '/target.txt', link = root + '/target-link'; fs.writeFileSync(target, 'data'); fs.symlinkSync(target, link); fs.utimesSync(target, 100, 200); fs.lutimesSync(link, 300, 400); var sync = fs.lstatSync(link); await new Promise(function(resolve, reject) { fs.lutimes(link, 500, 600, function(error) { error ? reject(error) : resolve(); }); }); var callback = await promises.lstat(link); await promises.lutimes(link, new Date(700000), new Date(800000)); var promised = await promises.lstat(link), followed = await promises.stat(link); return [Math.round(sync.atimeMs), Math.round(sync.mtimeMs), Math.round(callback.atimeMs), Math.round(callback.mtimeMs), Math.round(promised.atimeMs), Math.round(promised.mtimeMs), Math.round(followed.atimeMs), Math.round(followed.mtimeMs)]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_lutimes_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsLutimes = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsLutimes").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[300000,400000,500000,600000,700000,800000,100000,200000]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_copy_exclusive_and_forced_removal_match_node_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_operation_options");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source.txt', destination = root + '/destination.txt', missing = root + '/missing'; fs.writeFileSync(source, 'source'); fs.writeFileSync(destination, 'existing'); var syncError, callbackError, promiseError, missingError; try { fs.copyFileSync(source, destination, fs.constants.COPYFILE_EXCL); } catch (error) { syncError = [error.code, error.path, error.syscall]; } await new Promise(function(resolve) { fs.copyFile(source, destination, fs.constants.COPYFILE_EXCL, function(error) { callbackError = [error.code, error.path, error.syscall]; resolve(); }); }); try { await promises.copyFile(source, destination, fs.constants.COPYFILE_EXCL); } catch (error) { promiseError = [error.code, error.path, error.syscall]; } fs.rmSync(missing, { force: true }); await new Promise(function(resolve, reject) { fs.rm(missing, { force: true }, function(error) { error ? reject(error) : resolve(); }); }); await promises.rm(missing, { force: true }); try { fs.rmSync(missing); } catch (error) { missingError = error.code; } return [syncError, callbackError, promiseError, fs.readFileSync(destination, 'utf8'), missingError]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_operation_options_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsOperationOptions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsOperationOptions").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let destination = dir.join("destination.txt").to_string_lossy().into_owned();
    assert_eq!(
        result,
        format!(
            r#"[["EEXIST","{0}","copyfile"],["EEXIST","{0}","copyfile"],["EEXIST","{0}","copyfile"],"existing","ENOENT"]"#,
            destination
        )
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_readdir_supports_recursive_and_buffer_results() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_recursive_readdir");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var work = root + '/tree'; fs.mkdirSync(work + '/nested/deep', { recursive: true }); fs.writeFileSync(work + '/root.txt', 'root'); fs.writeFileSync(work + '/nested/value.txt', 'value'); fs.writeFileSync(work + '/nested/deep/end.txt', 'end'); fs.symlinkSync(work + '/nested', work + '/linked'); var sync = fs.readdirSync(work, { recursive: true }).sort(); var callback = await new Promise(function(resolve, reject) { fs.readdir(work, { recursive: true, withFileTypes: true }, function(error, entries) { if (error) return reject(error); resolve(entries.map(function(entry) { return [entry.name, entry.parentPath.slice(work.length), entry.isFile(), entry.isDirectory(), entry.isSymbolicLink()]; }).sort()); }); }); var buffers = (await promises.readdir(work, { recursive: true, encoding: 'buffer' })).map(function(value) { return [Buffer.isBuffer(value), value.toString()]; }).sort(function(left, right) { return left[1].localeCompare(right[1]); }); return [sync, callback, buffers]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_recursive_readdir_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsRecursiveReaddir = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsRecursiveReaddir").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let values = parsed.as_array().unwrap();
    assert_eq!(
        values[0],
        serde_json::json!([
            "linked",
            "nested",
            "nested/deep",
            "nested/deep/end.txt",
            "nested/value.txt",
            "root.txt"
        ])
    );
    assert_eq!(values[1].as_array().unwrap().len(), 6);
    assert_eq!(
        values[1][0],
        serde_json::json!(["deep", "/nested", false, true, false])
    );
    assert_eq!(
        values[1][1],
        serde_json::json!(["end.txt", "/nested/deep", true, false, false])
    );
    assert_eq!(
        values[1][2],
        serde_json::json!(["linked", "", false, false, true])
    );
    assert!(values[2]
        .as_array()
        .unwrap()
        .iter()
        .all(|entry| entry[0] == serde_json::json!(true)));
    assert_eq!(values[2].as_array().unwrap().len(), 6);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_write_flags_honor_append_and_exclusive_creation() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_write_flags");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var path = root + '/flags.txt', exclusive = root + '/exclusive.txt', opened = root + '/opened.txt'; fs.writeFileSync(path, 'one'); fs.writeFileSync(path, '-two', { flag: 'a' }); await new Promise(function(resolve, reject) { fs.writeFile(path, '-three', { flag: 'a' }, function(error) { error ? reject(error) : resolve(); }); }); await promises.writeFile(path, '-four', { flag: 'a' }); fs.appendFileSync(exclusive, 'created', { flag: 'ax' }); var errors = []; try { fs.writeFileSync(path, 'lost', { flag: 'wx' }); } catch (error) { errors.push([error.code, error.path, error.syscall]); } await new Promise(function(resolve) { fs.appendFile(exclusive, 'lost', { flag: 'ax' }, function(error) { errors.push([error.code, error.path, error.syscall]); resolve(); }); }); try { await promises.writeFile(path, 'lost', { flag: 'ax' }); } catch (error) { errors.push([error.code, error.path, error.syscall]); } try { fs.openSync(path, 'wx'); } catch (error) { errors.push([error.code, error.path, error.syscall]); } try { await promises.open(path, 'ax'); } catch (error) { errors.push([error.code, error.path, error.syscall]); } var fd = fs.openSync(opened, 'wx'); fs.closeSync(fd); return [fs.readFileSync(path, 'utf8'), fs.readFileSync(exclusive, 'utf8'), fs.existsSync(opened), fs.statSync(opened).size, errors]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_write_flags_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsWriteFlags = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsWriteFlags").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let path = dir.join("flags.txt").to_string_lossy().into_owned();
    let exclusive = dir.join("exclusive.txt").to_string_lossy().into_owned();
    assert_eq!(
        result,
        format!(
            r#"["one-two-three-four","created",true,0,[["EEXIST","{0}","open"],["EEXIST","{1}","open"],["EEXIST","{0}","open"],["EEXIST","{0}","open"],["EEXIST","{0}","open"]]]"#,
            path, exclusive
        )
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_numeric_open_flags_behave_like_their_string_equivalents() {
    // Real Node accepts `fs.constants.O_*` bitmask flags anywhere a
    // string flag ('w'/'a'/'r+'/...) is accepted -- `createWriteStream`,
    // `openSync`, and `writeFileSync`'s `flag` option included. thaw's
    // own polyfill only ever compared `flags.charAt(0)` against a
    // string, so a numeric flags value (`String(577)` -> `"577"`) never
    // matched 'w'/'a' and fell through to a stray `stat` check instead
    // of creating the file -- a real, reproducible `tar.extract()`
    // failure: tar's own `getWriteFlag(size)` returns the numeric
    // `O_TRUNC|O_CREAT|O_WRONLY` combination (577) rather than the
    // string `"w"`, so every extracted file's `WriteStream` threw
    // ENOENT statting a destination that legitimately doesn't exist yet.
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_numeric_flags");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'); module.exports = async function (root) { var c = fs.constants, writeFlags = c.O_TRUNC | c.O_CREAT | c.O_WRONLY, appendFlags = c.O_APPEND | c.O_CREAT | c.O_WRONLY, streamed = root + '/streamed.txt', opened = root + '/opened.txt', appended = root + '/appended.txt'; await new Promise(function(resolve, reject) { var writer = fs.createWriteStream(streamed, { flags: writeFlags }); writer.on('error', reject); writer.on('finish', resolve); writer.end('hello'); }); var fd = fs.openSync(opened, writeFlags); fs.closeSync(fd); fs.writeFileSync(appended, 'one'); fs.writeFileSync(appended, '-two', { flag: appendFlags }); return [fs.readFileSync(streamed, 'utf8'), fs.existsSync(opened), fs.readFileSync(appended, 'utf8')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_numeric_flags_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsNumericFlags = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsNumericFlags").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["hello",true,"one-two"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_open_and_file_options_preserve_independent_flags() {
    // Unrun regression: numeric O_CREAT must not imply O_TRUNC, and the
    // file APIs must honor the caller's flag without closing caller-owned fds.
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_independent_open_flags");
    fs::write(
        dir.join("index.js"),
        r#"var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function(root) { var c = fs.constants, path = root + '/flag.txt', created = root + '/created.txt', missing = root + '/missing.txt'; fs.writeFileSync(path, 'abcdef'); var first = fs.openSync(path, c.O_RDWR | c.O_CREAT); fs.closeSync(first); var preserved = fs.readFileSync(path, 'utf8'); await new Promise(function(resolve, reject) { fs.open(path, c.O_RDWR | c.O_CREAT, function(error, fd) { if (error) return reject(error); fs.closeSync(fd); resolve(); }); }); var opened = await promises.open(path, c.O_RDWR | c.O_CREAT); await opened.close(); fs.writeFileSync(path, 'XY', { flag: 'r+' }); await new Promise(function(resolve, reject) { fs.writeFile(path, 'Z', { flag: 'r+' }, function(error) { error ? reject(error) : resolve(); }); }); await promises.writeFile(path, '!', { flag: 'r+' }); var beforeTrunc = fs.readFileSync(path, 'utf8'), invalid; try { fs.readFileSync(path, { flag: 'w', encoding: 'invalid-encoding' }); } catch(error) { invalid = error.name; } var afterInvalid = await promises.readFile(path, { flag: 'r', encoding: 'utf8' }), callbackRead = await new Promise(function(resolve, reject) { fs.readFile(path, { flag: 'r', encoding: 'utf8' }, function(error, value) { error ? reject(error) : resolve(value); }); }); var missingCode; try { fs.writeFileSync(missing, 'x', { flag: 'r+' }); } catch(error) { missingCode = error.code; } var empty = fs.readFileSync(created, { flag: 'a+' }).length; var fd = fs.openSync(path, 'r+'); fs.writeFileSync(fd, 'X', { flag: 'w' }); var descriptorTail = fs.readFileSync(fd, 'utf8'); fs.closeSync(fd); var truncate = fs.openSync(path, c.O_WRONLY | c.O_TRUNC); fs.closeSync(truncate); var combo = root + '/append-truncate.txt'; fs.writeFileSync(combo, 'old'); var comboFd = fs.openSync(combo, c.O_WRONLY | c.O_APPEND | c.O_TRUNC); fs.writeFileSync(comboFd, 'new'); fs.closeSync(comboFd); return [preserved, beforeTrunc, invalid, afterInvalid, callbackRead, missingCode, empty, fs.existsSync(created), descriptorTail, fs.readFileSync(path, 'utf8'), fs.readFileSync(combo, 'utf8')]; };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_independent_open_flags_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsIndependentFlags = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments = CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsIndependentFlags").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["abcdef","!Ycdef","TypeError","!Ycdef","!Ycdef","ENOENT",0,true,"Ycdef","","new"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_stream_positions_reject_invalid_original_values_before_open() {
    // Unrun regression: constructor argument validation must happen before
    // default 'w' can truncate an existing file or create a new one.
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_stream_positions");
    fs::write(
        dir.join("index.js"),
        r#"var fs = require('node:fs'); module.exports = async function(root) { var path = root + '/original.txt', absent = root + '/absent.txt'; fs.writeFileSync(path, 'abcdef'); var writeErrors = ['0', null, -1, 1.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1].map(function(start) { try { fs.createWriteStream(path, { start: start }); return 'none'; } catch(error) { return error.name; } }); var missingError; try { fs.createWriteStream(absent, { start: '0' }); } catch(error) { missingError = error.name; } var readErrors = []; [{start:'1'}, {end:-1}, {end:'3'}, {start:4,end:2}].forEach(function(options) { try { fs.createReadStream(path, options); readErrors.push('none'); } catch(error) { readErrors.push(error.name); } }); var preserved = fs.readFileSync(path, 'utf8'), missing = fs.existsSync(absent), handle = await fs.promises.open(path, 'r+'), handleError; try { handle.createWriteStream({start:null}); } catch(error) { handleError = error.name; } var handleOpen = !handle.closed; await handle.close(); await new Promise(function(resolve, reject) { var writer = fs.createWriteStream(path, {flags:'r+', start:2}); writer.on('error',reject); writer.on('finish',resolve); writer.end('XY'); }); var range = await new Promise(function(resolve,reject) { var reader = fs.createReadStream(path, {start:1,end:3}), chunks=[]; reader.on('data',function(chunk) { chunks.push(chunk.toString()); }); reader.on('error',reject); reader.on('end',function() { resolve(chunks.join('')); }); }); return [writeErrors,missingError,readErrors,preserved,missing,handleError,handleOpen,fs.readFileSync(path,'utf8'),range]; };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_stream_positions_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsStreamPositions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments = CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsStreamPositions").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[["TypeError","TypeError","RangeError","RangeError","RangeError","RangeError","RangeError"],"TypeError",["TypeError","RangeError","TypeError","RangeError"],"abcdef",false,"TypeError",true,"abXYef","bXY"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[cfg(unix)]
#[test]
fn fs_stat_no_entry_option_respects_sync_and_async_boundaries() {
    // Unrun regression: sync lstat suppresses only ENOENT; async lstat never
    // suppresses missing entries, while stat suppresses ENOENT and ENOTDIR.
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_stat_no_entry");
    fs::write(
        dir.join("index.js"),
        r#"var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function(root) { var plain = root + '/plain.txt', missing = root + '/missing.txt', link = root + '/dangling', nested = plain + '/child'; fs.writeFileSync(plain,'payload'); fs.symlinkSync('missing.txt',link); function callbackResult(method,path,options) { return new Promise(function(resolve) { method(path,options,function(error,value) { resolve([error ? error.code : null,value === undefined]); }); }); } var syncMissing = [fs.statSync(missing,{throwIfNoEntry:false}) === undefined,fs.lstatSync(missing,{throwIfNoEntry:false}) === undefined], asyncStat = [await callbackResult(fs.stat,missing,{throwIfNoEntry:false}),await promises.stat(missing,{throwIfNoEntry:false}) === undefined], asyncLstat = await callbackResult(fs.lstat,missing,{throwIfNoEntry:false}), promiseLstat; try { await promises.lstat(missing,{throwIfNoEntry:false}); } catch(error) { promiseLstat = error.code; } var nestedStat = fs.statSync(nested,{throwIfNoEntry:false}) === undefined, nestedLstat; try { fs.lstatSync(nested,{throwIfNoEntry:false}); } catch(error) { nestedLstat = error.code; } var asyncNested = [await callbackResult(fs.stat,nested,{throwIfNoEntry:false}),await promises.stat(nested,{throwIfNoEntry:false}) === undefined,await callbackResult(fs.lstat,nested,{throwIfNoEntry:false})], dangling = [fs.statSync(link,{throwIfNoEntry:false}) === undefined,fs.lstatSync(link,{throwIfNoEntry:false}).isSymbolicLink()], successful = [typeof fs.statSync(Buffer.from(plain),{bigint:true}).size,fs.statSync(new URL('file://' + plain)).size,fs.lstatSync(link,{bigint:true}).isSymbolicLink()]; var host = globalThis.__thaw_fs, otherErrors = []; try { globalThis.__thaw_fs = function(operation,path,value,recursive) { if (operation === 'stat' && path === plain) return JSON.stringify({ok:false,code:'EACCES',message:'denied'}); if (operation === 'lstat' && path === plain) return JSON.stringify({ok:false,code:'EINVAL',message:'invalid'}); return host(operation,path,value,recursive); }; try { fs.statSync(plain,{throwIfNoEntry:false}); } catch(error) { otherErrors.push(error.code); } try { fs.lstatSync(plain,{throwIfNoEntry:false}); } catch(error) { otherErrors.push(error.code); } } finally { globalThis.__thaw_fs = host; } var getterError = new Error('bigint getter'); getterError.code = 'ENOENT'; var getterOptions = {throwIfNoEntry:false}; Object.defineProperty(getterOptions,'bigint',{get:function() { throw getterError; }}); var getterChecks = []; try { fs.statSync(plain,getterOptions); } catch(error) { getterChecks.push(error === getterError); } getterChecks.push(await new Promise(function(resolve) { fs.stat(plain,getterOptions,function(error) { resolve(error === getterError); }); })); try { await promises.stat(plain,getterOptions); } catch(error) { getterChecks.push(error === getterError); } var conversionError = new Error('path conversion'); conversionError.code = 'ENOENT'; var conversionPassed = false; try { fs.statSync({toString:function() { throw conversionError; }},{throwIfNoEntry:false}); } catch(error) { conversionPassed = error === conversionError; } return [syncMissing,asyncStat,asyncLstat,promiseLstat,nestedStat,nestedLstat,asyncNested,dangling,successful,otherErrors,getterChecks,conversionPassed]; };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_stat_no_entry_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsStatNoEntry = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments = CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsStatNoEntry").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[[true,true],[[null,true],true],["ENOENT",true],"ENOENT",true,"ENOTDIR",[[null,true],true,["ENOTDIR",true]],[true,true],["bigint",7,true],["EACCES","EINVAL"],[true,true,true],true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_creation_apis_honor_requested_modes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_creation_modes");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var sync = root + '/sync.txt', callback = root + '/callback.txt', promised = root + '/promised.txt', opened = root + '/opened.txt', asyncOpened = root + '/async-opened.txt', streamed = root + '/streamed.txt', directory = root + '/directory'; fs.writeFileSync(sync, 'sync', { mode: 384 }); await new Promise(function(resolve, reject) { fs.appendFile(callback, 'callback', { mode: '600' }, function(error) { error ? reject(error) : resolve(); }); }); await promises.writeFile(promised, 'promised', { mode: 384 }); var fd = fs.openSync(opened, 'w', 384); fs.closeSync(fd); var handle = await promises.open(asyncOpened, 'w', 384); await handle.close(); fs.mkdirSync(directory, { mode: 448 }); var writer = fs.createWriteStream(streamed, { mode: 384 }); await new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); writer.end('stream'); }); var numericOpened = root + '/numeric-opened.txt', numericWritten = root + '/numeric-written.txt', numericFlags = fs.constants.O_CREAT | fs.constants.O_RDWR; var numericFd = fs.openSync(numericOpened, numericFlags, 384); fs.closeSync(numericFd); fs.writeFileSync(numericWritten, 'created', { flag: numericFlags, mode: 384 }); var modes = [sync, callback, promised, opened, asyncOpened, streamed, directory, numericOpened, numericWritten].map(function(path) { return fs.statSync(path).mode & 511; }); var invalid = []; try { fs.chmodSync(sync, '700junk'); } catch (error) { invalid.push(error.name); } try { fs.writeFileSync(root + '/bad.txt', 'bad', { mode: 'nope' }); } catch (error) { invalid.push(error.name); } try { fs.mkdirSync(root + '/bad', { mode: '7x' }); } catch (error) { invalid.push(error.name); } return modes.concat(invalid, fs.existsSync(root + '/bad.txt'), fs.existsSync(root + '/bad')); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_creation_modes_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsCreationModes = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsCreationModes").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[384,384,384,384,384,384,448,384,384,"TypeError","TypeError","TypeError",false,false]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_truncate_and_file_handle_sync_methods_work() {
    use std::ffi::{CStr, CString};
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_registry("builtin_fs_truncate_sync");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { fs.writeFileSync(path, 'abcdef'); fs.truncateSync(path, 3); await new Promise(function(resolve, reject) { fs.truncate(path, 5, function(error) { error ? reject(error) : resolve(); }); }); await promises.truncate(path, 4); var invalidCode; try { fs.truncateSync(path, -1); } catch (error) { invalidCode = error.name; } var preserved = fs.readFileSync(path).toString('hex'); var handle = await promises.open(path, 'r+'); await handle.chmod(384); await handle.sync(); await handle.datasync(); await handle.close(); var closedCode; try { await handle.sync(); } catch (error) { closedCode = error.code; } return [fs.statSync(path).size, fs.readFileSync(path).toString('hex'), closedCode, invalidCode, preserved]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_truncate_sync_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsTruncateSync = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("truncate.txt");
    let arguments =
        CString::new(serde_json::to_string(&[path.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsTruncateSync").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[4,"61626300","EBADF","RangeError","61626300"]"#);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_bigint_stats_include_host_identity_and_nanoseconds() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_bigint_stats");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { fs.writeFileSync(path, 'bigint'); var sync = fs.statSync(path, { bigint: true }), callback = await new Promise(function(resolve, reject) { fs.stat(path, { bigint: true }, function(error, value) { error ? reject(error) : resolve(value); }); }), promised = await promises.lstat(path, { bigint: true }), fd = fs.openSync(path, 'r'), descriptor = fs.fstatSync(fd, { bigint: true }); fs.closeSync(fd); var handle = await promises.open(path, 'r'), handled = await handle.stat({ bigint: true }); await handle.close(); function summarize(value) { return [typeof value.size, value.size.toString(), typeof value.ino, value.ino > 0n, typeof value.blocks, typeof value.atimeMs, typeof value.atimeNs, value.atimeNs / 1000000n === value.atimeMs, value.atime instanceof Date]; } return [summarize(sync), callback.size === sync.size, promised.ino === sync.ino, descriptor.dev === sync.dev, handled.nlink === sync.nlink]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_bigint_stats_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsBigintStats = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("bigint.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsBigintStats").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["bigint","6","bigint",true,"bigint","bigint","bigint",true,true],true,true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_bigint_stats_keep_exact_wire_values_across_consumers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_bigint_exact_wire");
    fs::write(
        dir.join("index.js"),
        "var fs = require('node:fs'); module.exports = async function(path) { fs.writeFileSync(path, 'x'); var original = globalThis.__thaw_fs, exact = { length: '9007199254740993', dev: '9007199254740995', ino: '9007199254740997', mode: '33188', nlink: '1', uid: '0', gid: '0', rdev: '0', blksize: '4096', blocks: '1', atimeNs: '-1', mtimeNs: '1000000000000000001', ctimeNs: '-1000001', birthtimeNs: '123456789' }, record = { ok: true, length: 9007199254740992, dev: 9007199254740996, ino: 9007199254740996, mode: 33188, nlink: 1, uid: 0, gid: 0, rdev: 0, blksize: 4096, blocks: 1, atimeMs: -0.000001, mtimeMs: 1000000000000, ctimeMs: -1.000001, birthtimeMs: 123.456789, file: true, directory: false, symlink: false, exact: exact }; globalThis.__thaw_fs = function(operation, filename, value, recursive) { if (operation === 'stat' || operation === 'lstat' || operation === 'fd_stat') return JSON.stringify(record); return original(operation, filename, value, recursive); }; var fd, handle; try { var normal = fs.statSync(path), sync = fs.statSync(path, { bigint: true }), callback = await new Promise(function(resolve, reject) { fs.stat(path, { bigint: true }, function(error, value) { error ? reject(error) : resolve(value); }); }), promised = await fs.promises.lstat(path, { bigint: true }); fd = fs.openSync(path, 'r'); var descriptor = fs.fstatSync(fd, { bigint: true }); handle = await fs.promises.open(path, 'r'); var handled = await handle.stat({ bigint: true }), resolveChange, rejectChange, change = new Promise(function(resolve, reject) { resolveChange = resolve; rejectChange = reject; }), watcher = fs.watchFile(path, { bigint: true, interval: 1 }, function(current, previous) { resolveChange([current.mtimeNs, previous.mtimeNs]); }), watched = watcher._previous.stats, timeout = setTimeout(function() { rejectChange(new Error('watchFile missed nanosecond change')); }, 500); record.exact.mtimeNs = '1000000000000000002'; var changed = await change; clearTimeout(timeout); fs.unwatchFile(path); return [normal.size, sync.size.toString(), sync.dev.toString(), sync.ino.toString(), sync.atimeNs.toString(), sync.atimeMs.toString(), sync.mtimeNs.toString(), sync.mtimeMs.toString(), sync.ctimeNs.toString(), sync.ctimeMs.toString(), sync.birthtimeNs.toString(), sync.birthtimeMs.toString(), callback.ino === sync.ino, promised.size === sync.size, descriptor.dev === sync.dev, handled.mtimeNs === sync.mtimeNs, watched.ino === sync.ino, sync.atime instanceof Date, changed[0] === sync.mtimeNs + 1n && changed[1] === sync.mtimeNs]; } finally { fs.unwatchFile(path); if (fd !== undefined) fs.closeSync(fd); if (handle) await handle.close(); globalThis.__thaw_fs = original; } };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_bigint_exact_wire_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsBigintExactWire = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("exact.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let function = CString::new("exerciseFsBigintExactWire").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[9007199254740992,"9007199254740993","9007199254740995","9007199254740997","-1","0","1000000000000000001","1000000000000","-1000001","-1","123456789","123",true,true,true,true,true,true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_promises_watch_iterates_changes_and_honors_abort() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_promises_watch");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var work = root + '/watched'; fs.mkdirSync(work); var iterator = promises.watch(work, { interval: 1, encoding: 'buffer' }); setTimeout(function() { fs.writeFileSync(work + '/value.txt', 'value'); }, 3); var event = await iterator.next(), returned = await iterator.return(), after = await iterator.next(); var controller = new AbortController(), aborted = promises.watch(work, { interval: 1, signal: controller.signal }), pending = aborted.next(), abortName, abortCause; controller.abort('reason'); try { await pending; } catch (error) { abortName = error.name; abortCause = error.cause; } return [event.done, event.value.eventType, Buffer.isBuffer(event.value.filename), event.value.filename.toString(), returned.done, after.done, abortName, abortCause]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_promises_watch_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsPromisesWatch = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsPromisesWatch").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[false,"rename",true,"value.txt",true,true,"AbortError","reason"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}


#[test]
fn fs_retained_descriptors_survive_rename_unlink_and_enforce_native_flags() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_retained_descriptors");
    fs::write(
        dir.join("index.js"),
        "var fs = require('node:fs'); module.exports = function(root) { var path = root + '/original.txt', renamed = root + '/renamed.txt', created = root + '/readonly.txt', appended = root + '/append.txt'; fs.writeFileSync(path, 'abcdef'); var fd = fs.openSync(path, 'r+'), first = Buffer.alloc(2), second = Buffer.alloc(2), positioned = Buffer.alloc(1), last = Buffer.alloc(2); fs.readSync(fd, first, 0, 2, null); fs.renameSync(path, renamed); fs.unlinkSync(renamed); fs.readSync(fd, second, 0, 2, -1); fs.readSync(fd, positioned, { position: 0, length: 1 }); fs.readSync(fd, last, 0, 2, null); var size = fs.fstatSync(fd).size, blocks = fs.statfsSync(root).blocks; fs.fsyncSync(fd); fs.fdatasyncSync(fd); fs.closeSync(fd); var closed; try { fs.readSync(fd, Buffer.alloc(1)); } catch (error) { closed = error.code; } var readonly = fs.openSync(created, fs.constants.O_RDONLY | fs.constants.O_CREAT, 384), denied; try { fs.writeSync(readonly, Buffer.from('x')); } catch (error) { denied = error.code; } fs.closeSync(readonly); fs.writeFileSync(appended, 'one'); var appendFd = fs.openSync(appended, 'a+'); fs.writeSync(appendFd, '!', 0, 'utf8'); fs.closeSync(appendFd); return [first.toString(), second.toString(), positioned.toString(), last.toString(), size, Number.isFinite(blocks), closed, denied, fs.readFileSync(appended, 'utf8'), fs.existsSync(path), fs.existsSync(renamed), fs.existsSync(created)]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_retained_descriptors_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsRetainedDescriptors = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments = CString::new(
        serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap(),
    )
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsRetainedDescriptors").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["ab","cd","a","ef",6,true,"EBADF","EBADF","one!",false,false,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_descriptor_vectors_stop_after_short_write_and_read_options_size_buffer() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_fd_short_vector");
    fs::write(
        dir.join("index.js"),
        "var fs = require('node:fs'); module.exports = async function(path) { fs.writeFileSync(path, 'abcdef'); var handle = await fs.promises.open(path, 'r+'), sized = await handle.read({ offset: 2, length: 4, position: 0 }), empty = await handle.read({ offset: 3, length: 0, position: 0 }), host = globalThis.__thaw_fs, writes = [], count; try { globalThis.__thaw_fs = function(operation, target, value, recursive) { if (operation === 'fd_write') { writes.push(value); return JSON.stringify({ ok: true, length: 1 }); } return host(operation, target, value, recursive); }; count = fs.writevSync(handle.fd, [Buffer.from('abc'), Buffer.from('def')]); } finally { globalThis.__thaw_fs = host; await handle.close(); } return [sized.bytesRead, sized.buffer.length, sized.buffer.toString('hex'), empty.bytesRead, empty.buffer.length, count, writes.length, writes[0]]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_fd_short_vector_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsFdShortVector = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let path = dir.join("short-vector.txt").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsFdShortVector").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[4,6,"000061626364",0,3,1,1,"-1:616263"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_watchers_track_missing_bigint_and_timer_lifetime() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_watch_missing_bigint");
    fs::write(
        dir.join("index.js"),
        "var fs = require('node:fs'); module.exports = async function(root) { var path = root + '/later.txt', seen = [], watcher; var done = new Promise(function(resolve) { watcher = fs.watchFile(path, { interval: 1, bigint: true }, function(now, before) { seen.push([String(before.size), String(now.size), typeof now.size, String(now.mtimeNs)]); if (seen.length === 1) fs.writeFileSync(path, 'x'); else if (seen.length === 2) fs.unlinkSync(path); else if (seen.length === 3) fs.writeFileSync(path, 'long'); else { fs.unwatchFile(path); resolve(); } }); }); var initiallyRefed = watcher._timer.hasRef(); watcher.unref(); var unrefed = watcher._timer.hasRef(); watcher.ref(); var rerefed = watcher._timer.hasRef(); await done; fs.symlinkSync(root, root + '/cycle'); var controller = new AbortController(), fsWatcher = fs.watch(root, { recursive: true, signal: controller.signal, persistent: false }), timerUnrefed = fsWatcher._timer.hasRef(); fsWatcher.ref(); var timerRefed = fsWatcher._timer.hasRef(); fsWatcher.close(); return [seen, initiallyRefed, unrefed, rerefed, watcher._timer, timerUnrefed, timerRefed, fsWatcher._timer]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_watch_missing_bigint_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsWatchMissing = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments = CString::new(
        serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap(),
    )
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsWatchMissing").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let events = parsed[0].as_array().unwrap();
    assert_eq!(events.len(), 4);
    assert_eq!(events[0], serde_json::json!(["0", "0", "bigint", "0"]));
    assert_eq!(events[1][0], "0");
    assert_eq!(events[1][1], "1");
    assert_eq!(events[1][2], "bigint");
    assert_eq!(events[2], serde_json::json!(["1", "0", "bigint", "0"]));
    assert_eq!(events[3][0], "0");
    assert_eq!(events[3][1], "4");
    assert_eq!(events[3][2], "bigint");
    assert_eq!(parsed[1], true);
    assert_eq!(parsed[2], false);
    assert_eq!(parsed[3], true);
    assert!(parsed[4].is_null());
    assert_eq!(parsed[5], false);
    assert_eq!(parsed[6], true);
    assert!(parsed[7].is_null());
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_glob_parses_patterns_and_prunes_excluded_directories() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_glob_patterns");
    fs::write(
        dir.join("index.js"),
        "var fs = require('node:fs'); module.exports = async function(root) { var work = root + '/glob'; fs.mkdirSync(work + '/sub', { recursive: true }); fs.mkdirSync(work + '/private/sub', { recursive: true }); fs.mkdirSync(work + '/.hidden'); ['file1.js','file2.js','file3.js','file2.ts','.hidden.js','sub/main.js','private/secret.js','private/sub/deep.js','.hidden/inside.js','a.txt','é.txt','1.txt','file01.txt','file02.txt','file03.txt'].forEach(function(name) { fs.writeFileSync(work + '/' + name, name); }); var namedAlpha = fs.globSync('[[:alpha:]].txt', { cwd: work }).sort(), mixedClass = fs.globSync('[[:alpha:]0-9].txt', { cwd: work }).sort(), padded = fs.globSync('file{1..03}.txt', { cwd: work }).sort(), cls = fs.globSync('file[12].js', { cwd: work }).sort(), brace = fs.globSync('file{1..2}.js', { cwd: work }).sort(), ext = fs.globSync('file@(1|2).js', { cwd: work }).sort(), neg = fs.globSync('file!(3).js', { cwd: work }).sort(), dot = fs.globSync('**/*.js', { cwd: work }).sort(), explicitDot = fs.globSync('.*.js', { cwd: work }).sort(), absolute = fs.globSync(work + '/file[12].js').sort(), parent = fs.globSync('../file[12].js', { cwd: work + '/sub' }).sort(), relativeParent = fs.globSync('../file[12].js', { cwd: require('node:path').relative(process.cwd(), work + '/sub') }).sort(), dotted = fs.globSync('./file[12].js', { cwd: work }).sort(), backslash = fs.globSync('sub' + String.fromCharCode(92) + 'main.js', { cwd: work }), typed = fs.globSync('**/*.js', { cwd: work, withFileTypes: true, exclude: function(entry) { if (typeof entry.isDirectory !== 'function') throw new Error('exclude did not receive Dirent'); return entry.isDirectory() && entry.name === 'private'; } }).map(function(entry) { return entry.name; }).sort(); var original = fs.readdirSync, pruned, rootedPruned, typedRootPruned, ancestorPruned, typedAncestorPruned, absolutePruned; fs.readdirSync = function(path, options) { if (String(path) === work + '/private') throw new Error('excluded directory was traversed'); return original(path, options); }; try { pruned = fs.globSync('**/*.js', { cwd: work, exclude: ['private/**'] }).sort(); rootedPruned = fs.globSync('private/**/*.js', { cwd: work, exclude: ['private/**'] }); typedRootPruned = fs.globSync('private/**/*.js', { cwd: work, withFileTypes: true, exclude: function(entry) { if (typeof entry.isDirectory !== 'function') throw new Error('root exclude did not receive Dirent'); return entry.name === 'private'; } }); ancestorPruned = fs.globSync('private/sub/**/*.js', { cwd: work, exclude: ['private'] }); typedAncestorPruned = fs.globSync('private/sub/**/*.js', { cwd: work, withFileTypes: true, exclude: function(entry) { if (typeof entry.isDirectory !== 'function') throw new Error('ancestor exclude did not receive Dirent'); return entry.name === 'private'; } }); absolutePruned = fs.globSync(work + '/private/**/*.js', { cwd: work, exclude: ['private/**'] }); } finally { fs.readdirSync = original; } var callback = await new Promise(function(resolve, reject) { fs.glob('file{1,2}.js', { cwd: work }, function(error, values) { error ? reject(error) : resolve(values.sort()); }); }); var asyncValues = []; for await (var value of fs.promises.glob('file[12].js', { cwd: work })) asyncValues.push(value); return [cls, brace, ext, neg, dot, explicitDot, absolute, parent, typed, pruned, callback, asyncValues.sort(), backslash, dotted, relativeParent, rootedPruned, typedRootPruned, ancestorPruned, typedAncestorPruned, absolutePruned, namedAlpha, mixedClass, padded]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_glob_patterns_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsGlobPatterns = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments = CString::new(
        serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap(),
    )
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsGlobPatterns").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let two = serde_json::json!(["file1.js", "file2.js"]);
    for index in [0, 1, 2, 3, 10, 11] {
        assert_eq!(parsed[index], two);
    }
    assert_eq!(parsed[4], serde_json::json!(["file1.js", "file2.js", "file3.js", "private/secret.js", "private/sub/deep.js", "sub/main.js"]));
    assert_eq!(parsed[5], serde_json::json!([".hidden.js"]));
    let work = dir.join("glob").to_string_lossy().into_owned();
    assert_eq!(parsed[6], serde_json::json!([format!("{work}/file1.js"), format!("{work}/file2.js")]));
    assert_eq!(parsed[7], serde_json::json!(["../file1.js", "../file2.js"]));
    assert_eq!(parsed[8], serde_json::json!(["file1.js", "file2.js", "file3.js", "main.js"]));
    assert_eq!(parsed[9], serde_json::json!(["file1.js", "file2.js", "file3.js", "sub/main.js"]));
    assert_eq!(parsed[12], serde_json::json!(["sub/main.js"]));
    assert_eq!(parsed[13], two);
    assert_eq!(parsed[14], serde_json::json!(["../file1.js", "../file2.js"]));
    assert_eq!(parsed[15], serde_json::json!([]));
    assert_eq!(parsed[16], serde_json::json!([]));
    assert_eq!(parsed[17], serde_json::json!([]));
    assert_eq!(parsed[18], serde_json::json!([]));
    assert_eq!(parsed[19], serde_json::json!([]));
    assert_eq!(parsed[20], serde_json::json!(["a.txt", "é.txt"]));
    assert_eq!(parsed[21], serde_json::json!(["1.txt", "a.txt", "é.txt"]));
    assert_eq!(parsed[22], serde_json::json!(["file01.txt", "file02.txt", "file03.txt"]));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_recursive_mkdir_returns_first_created_path_across_api_styles() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_mkdir_result");
    fs::write(
        dir.join("index.js"),
        "var fs = require('node:fs'); module.exports = async function(root, checkMode) { var base = root + '/mkdir'; var first = fs.mkdirSync(base + '/one/two', { recursive: true, mode: 448 }); var existing = fs.mkdirSync(base + '/one/two', { recursive: true }); var nonrecursive = fs.mkdirSync(base + '/flat'); var promised = await fs.promises.mkdir(base + '/three/four', { recursive: true }); var callback = await new Promise(function(resolve, reject) { fs.mkdir(base + '/five/six', { recursive: true }, function(error, created) { error ? reject(error) : resolve([created, arguments.length]); }); }); var callbackFlat = await new Promise(function(resolve, reject) { fs.mkdir(base + '/plain', function(error) { error ? reject(error) : resolve(arguments.length); }); }); fs.writeFileSync(base + '/file', 'x'); var fileCode, parentCode; try { fs.mkdirSync(base + '/file', { recursive: true }); } catch (error) { fileCode = error.code; } try { fs.mkdirSync(base + '/file/child', { recursive: true }); } catch (error) { parentCode = error.code; } return [first, existing === undefined, nonrecursive === undefined, promised, callback, callbackFlat, fileCode, parentCode, fs.existsSync(base + '/one/two'), (!checkMode || (fs.statSync(base + '/one').mode & 63) === 0)]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_mkdir_result_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsMkdirResult = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments =
        CString::new(serde_json::to_string(&(dir.to_string_lossy().into_owned(), cfg!(unix))).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseFsMkdirResult").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    let base = dir.join("mkdir").to_string_lossy().into_owned();
    assert_eq!(parsed, serde_json::json!([
        base.clone(),
        true,
        true,
        format!("{base}/three"),
        [format!("{base}/five"), 2],
        1,
        "EEXIST",
        "ENOTDIR",
        true,
        true,
    ]));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[cfg(unix)]
#[test]
fn fs_readdir_names_do_not_require_search_permission() {
    use std::ffi::{CStr, CString};
    use std::os::unix::fs::PermissionsExt;

    let dir = temp_registry("builtin_fs_readdir_names_no_search");
    fs::write(
        dir.join("index.js"),
        "var fs = require('node:fs'); module.exports = async function(path) { var sync = fs.readdirSync(path).sort(); var callback = await new Promise(function(resolve, reject) { fs.readdir(path, function(error, names) { error ? reject(error) : resolve(names.sort()); }); }); var promised = (await fs.promises.readdir(path)).sort(); var typeReads = 0, recursiveReads = 0, changing = { get withFileTypes() { return ++typeReads > 1; }, get recursive() { return ++recursiveReads > 1; } }; var changingNames = fs.readdirSync(path, changing).sort(); return [sync, callback, promised, changingNames, typeReads, recursiveReads]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_readdir_names_no_search_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsReaddirNames = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);

    let locked = dir.join("locked");
    fs::create_dir(&locked).unwrap();
    fs::write(locked.join("entry.txt"), "data").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o400)).unwrap();
    let function = CString::new("exerciseFsReaddirNames").unwrap();
    let arguments = CString::new(
        serde_json::to_string(&[locked.to_string_lossy().into_owned()]).unwrap(),
    )
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o700)).unwrap();
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed, serde_json::json!([["entry.txt"], ["entry.txt"], ["entry.txt"], ["entry.txt"], 1, 1]));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[cfg(unix)]
#[test]
fn fs_raw_filename_bytes_survive_readdir_and_path_consumers() {
    use std::ffi::{CStr, CString, OsString};
    use std::os::unix::ffi::OsStringExt;
    let dir = temp_registry("builtin_fs_raw_names");
    fs::write(
        dir.join("index.js"),
        "var fs = require('node:fs'); module.exports = async function(root) { var base = Buffer.from(root + '/'), rawFile = Buffer.concat([base, Buffer.from([255])]), rawDir = Buffer.concat([base, Buffer.from([254])]), keep = function(values) { return values.map(function(value) { return Buffer.isBuffer(value) ? value.toString('hex') : value; }).filter(function(value) { return value === 'fe' || value === 'ff'; }).sort(); }; var plain = keep(fs.readdirSync(root, { encoding: 'buffer' })), hex = keep(fs.readdirSync(root, { encoding: 'hex' })), recursive = fs.readdirSync(root, { recursive: true, encoding: 'buffer' }).map(function(value) { return value.toString('hex'); }).filter(function(value) { return value === 'fe' || value === 'ff' || value === 'fe2ffd'; }).sort(), typed = fs.readdirSync(root, { withFileTypes: true, encoding: 'buffer' }).filter(function(entry) { return entry.name[0] >= 254; }).map(function(entry) { return [entry.name.toString('hex'), entry.isFile(), entry.isDirectory()]; }).sort(function(a,b) { return a[0].localeCompare(b[0]); }), opened = fs.opendirSync(root, { encoding: 'buffer' }), dirNames = [], entry; while ((entry = opened.readSync()) !== null) if (entry.name[0] >= 254) dirNames.push(entry.name.toString('hex')); opened.closeSync(); dirNames.sort(); var rawDirent = fs.readdirSync(root, { withFileTypes: true, encoding: 'buffer' }).find(function(entry) { return entry.name[0] === 254; }), mutableRawPath = rawDirent._rawPath; mutableRawPath.fill(0); var heldRawPath = rawDirent._rawPath.subarray(-1).toString('hex') === 'fe'; var globbed = fs.globSync('*', { cwd: Buffer.from(root), withFileTypes: true }).filter(function(entry) { return entry._rawPath && entry.name.length === 1 && entry.name.charCodeAt(0) === 65533; }).map(function(entry) { return entry._rawPath.subarray(-1).toString('hex'); }).sort(); var callback = await new Promise(function(resolve, reject) { fs.readdir(root, { encoding: 'buffer' }, function(error, names) { error ? reject(error) : resolve(keep(names)); }); }), promised = keep(await fs.promises.readdir(root, { encoding: 'buffer' })); fs.cpSync(rawDir, root + '/copied', { recursive: true }); var copied = fs.readdirSync(root + '/copied', { encoding: 'buffer' })[0].toString('hex'), streamText = await new Promise(function(resolve, reject) { var streamPath = Buffer.from(rawFile), reader = fs.createReadStream(streamPath), chunks = []; streamPath.fill(0); reader.path.fill(0); reader.on('data', function(chunk) { chunks.push(chunk); }); reader.on('error', reject); reader.on('end', function() { resolve(Buffer.concat(chunks).toString()); }); }), realLast = fs.realpathSync(rawFile, { encoding: 'buffer' }).subarray(-1).toString('hex'); fs.symlinkSync(rawFile, root + '/link'); var linkLast = fs.readlinkSync(root + '/link', { encoding: 'buffer' }).subarray(-1).toString('hex'); var prefix = Buffer.concat([base, Buffer.from('tmp-'), Buffer.from([250])]), temporary = fs.mkdtempSync(prefix, { encoding: 'buffer' }), prefixed = temporary.subarray(0, prefix.length).equals(prefix); fs.rmSync(temporary, { recursive: true }); var originalSize = fs.statSync(rawFile).size, originalText = fs.readFileSync(rawFile, 'utf8'), renamedPath = Buffer.concat([base, Buffer.from([251])]); fs.renameSync(rawFile, renamedPath); var renamed = fs.readFileSync(renamedPath, 'utf8') === 'F'; fs.renameSync(renamedPath, rawFile); var watched = await new Promise(function(resolve, reject) { var watcherPath = Buffer.from(root), watcher = fs.watch(watcherPath, { encoding: 'buffer', interval: 1 }, function(type, name) { if (name.toString('hex') !== 'ff') return; clearTimeout(timer); watcher.close(); resolve(name.toString('hex')); }), timer = setTimeout(function() { watcher.close(); reject(new Error('watch timeout')); }, 5000); watcherPath.fill(0); watcher.path.fill(0); fs.writeFileSync(rawFile, 'FF'); }); var writtenPath = Buffer.concat([base, Buffer.from([245])]), writeInput = Buffer.from(writtenPath), written = await new Promise(function(resolve, reject) { var writer = fs.createWriteStream(writeInput); writeInput.fill(0); writer.path.fill(0); writer.on('error', reject); writer.on('finish', function() { resolve(fs.readFileSync(writtenPath, 'utf8')); }); writer.end('W'); }); var mkdirSyncType = typeof fs.mkdirSync(Buffer.concat([base, Buffer.from([249]), Buffer.from('/child')]), { recursive: true }), mkdirCallbackType = await new Promise(function(resolve, reject) { fs.mkdir(Buffer.concat([base, Buffer.from([248]), Buffer.from('/child')]), { recursive: true }, function(error, created) { error ? reject(error) : resolve(typeof created); }); }), mkdirPromiseType = typeof (await fs.promises.mkdir(Buffer.concat([base, Buffer.from([247]), Buffer.from('/child')]), { recursive: true })); return [plain, hex, recursive, typed, dirNames, heldRawPath, globbed, callback, promised, originalSize, originalText, copied, streamText, realLast, linkLast, prefixed, renamed, watched, written, mkdirSyncType, mkdirCallbackType, mkdirPromiseType]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_raw_names_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let raw_file = dir.join(OsString::from_vec(vec![0xff]));
    let raw_directory = dir.join(OsString::from_vec(vec![0xfe]));
    fs::write(&raw_file, b"F").unwrap();
    fs::create_dir(&raw_directory).unwrap();
    fs::write(raw_directory.join(OsString::from_vec(vec![0xfd])), b"D").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseRawNames = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments = CString::new(
        serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap(),
    )
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseRawNames").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[["fe","ff"],["fe","ff"],["fe","fe2ffd","ff"],[["fe",false,true],["ff",true,false]],["fe","ff"],true,["fe","ff"],["fe","ff"],["fe","ff"],1,"F","fd","F","ff","ff",true,true,"ff","W","string","string","string"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_second_path_file_urls_survive_sync_callback_and_promise_routes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_second_file_urls");
    fs::write(
        dir.join("index.js"),
        "var fs = require('node:fs'); module.exports = async function(root) { var original = root + '/original.txt', sync = root + '/sync renamed.txt', callback = root + '/callback renamed.txt', promised = root + '/promise renamed.txt', copied = root + '/sync copied.txt', callbackCopy = root + '/callback copied.txt', promiseCopy = root + '/promise copied.txt', url = function(name) { return new URL('file://' + root + '/' + name); }; fs.writeFileSync(original, 'data'); fs.renameSync(original, url('sync%20renamed.txt')); fs.copyFileSync(sync, url('sync%20copied.txt')); await new Promise(function(resolve, reject) { fs.rename(sync, url('callback%20renamed.txt'), function(error) { error ? reject(error) : resolve(); }); }); await fs.promises.rename(callback, url('promise%20renamed.txt')); await new Promise(function(resolve, reject) { fs.copyFile(promised, url('callback%20copied.txt'), function(error) { error ? reject(error) : resolve(); }); }); await fs.promises.copyFile(promised, url('promise%20copied.txt')); fs.linkSync(promised, url('linked%20file.txt')); fs.symlinkSync(promised, url('linked%20symbol.txt')); return [fs.existsSync(original), fs.existsSync(sync), fs.existsSync(callback), fs.readFileSync(promised, 'utf8'), fs.readFileSync(copied, 'utf8'), fs.readFileSync(callbackCopy, 'utf8'), fs.readFileSync(promiseCopy, 'utf8'), fs.readFileSync(root + '/linked file.txt', 'utf8'), fs.readFileSync(root + '/linked symbol.txt', 'utf8')]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_second_file_urls_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseSecondFileUrls = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments = CString::new(
        serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap(),
    )
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseSecondFileUrls").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[false,false,false,"data","data","data","data","data","data"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[cfg(unix)]
#[test]
fn fs_invalid_byte_mkdtemp_disposables_remove_original_raw_paths() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_raw_disposable");
    fs::write(
        dir.join("index.js"),
        "var fs = require('node:fs'); module.exports = async function(root) { var base = Buffer.from(root + '/'), syncPrefix = Buffer.concat([base, Buffer.from('sync-'), Buffer.from([250])]), sync = fs.mkdtempDisposableSync(syncPrefix), syncRaw = Buffer.concat([syncPrefix, Buffer.from(sync.path.slice(-12))]), syncExists = fs.existsSync(syncRaw), syncType = typeof sync.path; sync.path = 'not the created path'; sync.remove(); sync.remove(); var syncRemoved = !fs.existsSync(syncRaw), asyncPrefix = Buffer.concat([base, Buffer.from('async-'), Buffer.from([249])]), disposable = await fs.promises.mkdtempDisposable(asyncPrefix), asyncRaw = Buffer.concat([asyncPrefix, Buffer.from(disposable.path.slice(-12))]), asyncExists = fs.existsSync(asyncRaw), asyncType = typeof disposable.path; disposable.path = 'not the created path'; var first = disposable.remove(), sameRemoval = first === disposable.remove(); await first; await disposable.remove(); return [syncType, syncExists, syncRemoved, asyncType, asyncExists, !fs.existsSync(asyncRaw), sameRemoval]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_raw_disposable_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseRawDisposable = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments = CString::new(
        serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap(),
    )
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseRawDisposable").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["string",true,true,"string",true,true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_deferred_buffer_paths_are_snapshotted() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_deferred_buffer_paths");
    fs::write(
        dir.join("index.js"),
        r#"var fs = require('node:fs'); module.exports = async function(root) {
            var original = root + '/original.txt';
            fs.writeFileSync(original, 'original');
            var readPath = Buffer.from(original), promised = fs.promises.readFile(readPath, 'utf8');
            readPath.fill(0);
            var promisedText = await promised;
            var callbackPath = Buffer.from(original), callbackText = new Promise(function(resolve, reject) {
                fs.readFile(callbackPath, 'utf8', function(error, text) { error ? reject(error) : resolve(text); });
            });
            callbackPath.fill(0);
            callbackText = await callbackText;
            var source = Buffer.from(original), destination = Buffer.from(root + '/copied.txt');
            var copied = fs.promises.cp(source, destination, { filter: function() { return Promise.resolve(true); } });
            source.fill(0); destination.fill(0);
            await copied;
            var mkdirPath = Buffer.from(root + '/made/child'), created = new Promise(function(resolve, reject) {
                fs.mkdir(mkdirPath, { recursive: true }, function(error, path) { error ? reject(error) : resolve(path); });
            });
            mkdirPath.fill(0);
            created = await created;
            var blobPath = Buffer.from(original), blob = fs.openAsBlob(blobPath);
            blobPath.fill(0);
            var blobText = await (await blob).text();
            var cwd = Buffer.from(root), iterator = fs.promises.glob('*.txt', { cwd: cwd });
            cwd.fill(0);
            var matches = [];
            for await (var match of iterator) matches.push(match);
            var callbackCwd = Buffer.from(root), callbackMatches = new Promise(function(resolve, reject) {
                fs.glob('*.txt', { cwd: callbackCwd }, function(error, values) { error ? reject(error) : resolve(values); });
            });
            callbackCwd.fill(0);
            callbackMatches = await callbackMatches;
            return [promisedText, callbackText, fs.readFileSync(root + '/copied.txt', 'utf8'), typeof created,
                fs.existsSync(root + '/made/child'), blobText, matches.includes('original.txt'),
                callbackMatches.includes('original.txt')];
        };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_fs_deferred_buffer_paths_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDeferredBufferPaths = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let arguments = CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
        .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseDeferredBufferPaths").unwrap().as_ptr(),
        arguments.as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["original","original","original","string",true,"original",true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_directory_iterator_closes_on_early_exit() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_dir_early_exit");
    fs::write(dir.join("index.js"), r#"var fs = require('node:fs'); module.exports = async function(root) {
        fs.mkdirSync(root + '/entries'); fs.writeFileSync(root + '/entries/a', 'a');
        var normal = fs.opendirSync(root + '/entries');
        for await (var item of normal) { break; }
        var failed = fs.opendirSync(root + '/entries');
        try { for await (var item of failed) { throw new Error('body'); } } catch (error) {}
        var iterator = normal[Symbol.asyncIterator](); await iterator.return();
        return [normal.closed, failed.closed, (await iterator.return()).done];
    };"#).unwrap();
    let modules = temp_registry("builtin_fs_dir_early_exit_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &dir, "index.js").unwrap();
    let source = CString::new(format!("globalThis.module = {{exports: {{}}}}; globalThis.exports = module.exports; {bundle} globalThis.exerciseDirExit = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseDirExit").unwrap();
    let arguments = CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap()).unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[true,true,true]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(modules);
}

#[test]
fn fs_bigint_statfs_keeps_exact_wire_values_across_consumers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_statfs_bigint_exact_wire");
    fs::write(
        dir.join("index.js"),
        r#"var fs = require('node:fs'); module.exports = async function(root) {
            var path = root + '/value.txt', original = globalThis.__thaw_fs;
            fs.writeFileSync(path, 'x');
            var exact = { type: '9007199254740993', bsize: '9007199254740995',
                blocks: '9007199254740997', bfree: '9007199254740999',
                bavail: '9007199254741001', files: '9007199254741003',
                ffree: '9007199254741005' };
            var record = { ok: true, type: 9007199254740992, bsize: 9007199254740996,
                blocks: 9007199254740996, bfree: 9007199254741000,
                bavail: 9007199254741000, files: 9007199254741004,
                ffree: 9007199254741004, exact: exact };
            globalThis.__thaw_fs = function(operation, filename, value, recursive) {
                if (operation === 'statfs' || operation === 'fd_statfs') return JSON.stringify(record);
                return original(operation, filename, value, recursive);
            };
            var handle;
            try {
                var normal = fs.statfsSync(root), sync = fs.statfsSync(root, { bigint: true });
                var callback = await new Promise(function(resolve, reject) {
                    fs.statfs(root, { bigint: true }, function(error, result) { error ? reject(error) : resolve(result); });
                });
                var promised = await fs.promises.statfs(root, { bigint: true });
                handle = await fs.promises.open(path, 'r');
                var descriptor = await handle.statfs({ bigint: true });
                var keys = ['type','bsize','blocks','bfree','bavail','files','ffree'];
                return [normal.type, typeof normal.blocks,
                    keys.every(function(key) { return sync[key].toString() === exact[key]; }),
                    keys.every(function(key) { return callback[key] === sync[key]; }),
                    keys.every(function(key) { return promised[key] === sync[key]; }),
                    keys.every(function(key) { return descriptor[key] === sync[key]; })];
            } finally {
                if (handle) await handle.close();
                globalThis.__thaw_fs = original;
            }
        };"#,
    )
    .unwrap();
    let modules = temp_registry("builtin_fs_statfs_bigint_exact_wire_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let source = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStatfsExact = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseStatfsExact").unwrap();
    let arguments = CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap()).unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[9007199254740992,\"number\",true,true,true,true]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(modules);
}

#[cfg(unix)]
#[test]
fn fs_cp_rejects_canonical_aliases_before_removal_or_recursive_creation() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_cp_alias_safety");
    fs::write(
        dir.join("index.js"),
        r#"var fs = require('node:fs'); module.exports = async function(root) {
            var source = root + '/source', file = source + '/keep.txt', alias = root + '/alias';
            fs.mkdirSync(source); fs.writeFileSync(file, 'intact'); fs.symlinkSync(source, alias);
            function code(work) { try { work(); return 'none'; } catch (error) { return error.code; } }
            var sync = code(function() { fs.cpSync(source, alias + '/sync', { recursive: true }); });
            var asyncCode; try { await fs.promises.cp(source, alias + '/async', { recursive: true }); }
            catch (error) { asyncCode = error.code; }
            var callbackCode = await new Promise(function(resolve) {
                fs.cp(source, alias + '/callback', { recursive: true }, function(error) { resolve(error && error.code); });
            });
            var dereferenced = code(function() {
                fs.cpSync(alias, source + '/from-alias', { recursive: true, dereference: true });
            });
            var sameAlias = code(function() { fs.cpSync(source, alias, { recursive: true }); });
            var hard = root + '/hardlink.txt'; fs.linkSync(file, hard);
            var hardCp = code(function() { fs.cpSync(file, hard); });
            var hardCopy = code(function() { fs.copyFileSync(file, hard); });
            var raw = code(function() {
                fs.cpSync(Buffer.from(source), Buffer.from(alias + '/raw'), { recursive: true });
            });
            var mutableSource = Buffer.from(file), mutableDestination = Buffer.from(root + '/filtered.txt');
            fs.cpSync(mutableSource, mutableDestination, { filter: function() {
                mutableSource.fill(0); mutableDestination.fill(0); return true;
            } });
            var filtered = fs.readFileSync(root + '/filtered.txt', 'utf8') === 'intact';
            fs.symlinkSync(source, source + '/cycle');
            var cycle = code(function() {
                fs.cpSync(source, root + '/cycle-copy', { recursive: true, dereference: true });
            });
            return [sync, asyncCode, callbackCode, dereferenced, sameAlias, hardCp,
                hardCopy, raw, cycle, fs.readFileSync(file, 'utf8'), fs.readFileSync(hard, 'utf8'),
                fs.existsSync(source + '/sync'), fs.existsSync(source + '/async'),
                fs.existsSync(source + '/callback'), fs.existsSync(source + '/from-alias'),
                fs.existsSync(source + '/raw'), fs.existsSync(root + '/cycle-copy/cycle'), filtered];
        };"#,
    )
    .unwrap();
    let modules = temp_registry("builtin_fs_cp_alias_safety_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &dir, "index.js").unwrap();
    let source = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseCpAliasSafety = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseCpAliasSafety").unwrap();
    let arguments = CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap()).unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let parsed: serde_json::Value = serde_json::from_str(&unsafe { CStr::from_ptr(result) }.to_string_lossy()).unwrap();
    let values = parsed.as_array().unwrap();
    assert!(values[..6].iter().all(|value| value == "EEXIST"));
    assert_eq!(values[7], "EEXIST");
    assert_eq!(values[8], "EEXIST");
    assert_eq!(values[9], "intact");
    assert_eq!(values[10], "intact");
    assert!(values[11..17].iter().all(|value| value == false));
    assert_eq!(values[17], true);
    assert!(values[6].as_str().is_some_and(|code| code != "none"));
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(modules);
}

#[test]
fn fs_write_stream_uses_configured_default_encoding() {
    // Unrun regression: the shared Writable write/end path must use the
    // filesystem stream's default encoding, with explicit writes overriding it.
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_write_stream_encoding");
    fs::write(dir.join("index.js"), r#"var fs = require('node:fs'); module.exports = async function(root) {
        var write = function(path, options, chunks) { return new Promise(function(resolve, reject) {
            var output = fs.createWriteStream(path, options); output.on('error', reject); output.on('finish', resolve);
            chunks(output);
        }); };
        var hex = root + '/hex', override = root + '/override', bytes = root + '/bytes', changed = root + '/changed';
        await write(hex, { encoding: 'hex' }, function(out) { out.end('6869'); });
        await write(override, { encoding: 'hex' }, function(out) { out.end('6869', 'utf8'); });
        await write(bytes, { encoding: 'hex' }, function(out) { out.end(Buffer.from('6869')); });
        await write(changed, { encoding: 'hex' }, function(out) { out.setDefaultEncoding('base64'); out.end('eQ=='); });
        var handlePath = root + '/handle', handle = await fs.promises.open(handlePath, 'w+');
        await new Promise(function(resolve, reject) { var out = handle.createWriteStream({ encoding: 'hex', autoClose: false }); out.on('error', reject); out.on('finish', resolve); out.end('6869'); });
        await handle.close();
        return [hex, override, bytes, changed, handlePath].map(function(path) { return fs.readFileSync(path, 'utf8'); });
    };"#).unwrap();
    let empty_node_modules = temp_registry("builtin_fs_write_stream_encoding_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFsWriteStreamEncoding = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseFsWriteStreamEncoding").unwrap();
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["hi","6869","6869","y","hi"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}
