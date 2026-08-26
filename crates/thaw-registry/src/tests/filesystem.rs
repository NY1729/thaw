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
            r#"["onetwo","onetwo",true,false,6,"value.txt",true,true,false,false,["ENOENT","{}/work/missing","read"]]"#,
            dir.to_string_lossy()
        )
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
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
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source', sync = root + '/sync', asyncPath = root + '/async', callbackPath = root + '/callback'; fs.mkdirSync(source + '/nested', { recursive: true }); fs.writeFileSync(source + '/nested/value.txt', 'copied'); fs.cpSync(source, sync, { recursive: true }); await promises.cp(source, asyncPath, { recursive: true }); await new Promise(function(resolve, reject) { fs.cp(source, callbackPath, { recursive: true }, function(error) { error ? reject(error) : resolve(); }); }); fs.writeFileSync(sync + '/nested/value.txt', 'kept'); fs.cpSync(source, sync, { recursive: true, force: false }); var existingCode; try { fs.cpSync(source, sync, { recursive: true, force: false, errorOnExist: true }); } catch (error) { existingCode = error.code; } return [fs.readFileSync(sync + '/nested/value.txt', 'utf8'), fs.readFileSync(asyncPath + '/nested/value.txt', 'utf8'), fs.readFileSync(callbackPath + '/nested/value.txt', 'utf8'), existingCode]; };",
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
    assert_eq!(result, r#"["kept","copied","copied","EEXIST"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn fs_cp_honors_filters_links_and_timestamp_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_cp_options");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var source = root + '/source', sync = root + '/sync', promised = root + '/promised', callback = root + '/callback'; fs.mkdirSync(source + '/nested', { recursive: true }); fs.writeFileSync(source + '/keep.txt', 'keep'); fs.writeFileSync(source + '/skip.txt', 'skip'); fs.writeFileSync(source + '/nested/value.txt', 'nested'); fs.symlinkSync('keep.txt', source + '/link'); fs.utimesSync(source + '/keep.txt', new Date(1000000), new Date(2000000)); var syncSeen = []; fs.cpSync(source, sync, { recursive: true, verbatimSymlinks: true, preserveTimestamps: true, filter: function(src) { syncSeen.push(src); return !src.endsWith('/skip.txt'); } }); var asyncSeen = []; await promises.cp(source, promised, { recursive: true, dereference: true, filter: async function(src) { asyncSeen.push(src); await Promise.resolve(); return !src.endsWith('/nested'); } }); var callbackSeen = []; await new Promise(function(resolve, reject) { fs.cp(source, callback, { recursive: true, filter: function(src) { callbackSeen.push(src); return Promise.resolve(!src.endsWith('/skip.txt')); } }, function(error) { error ? reject(error) : resolve(); }); }); var directoryCode; try { fs.cpSync(source, root + '/not-recursive'); } catch (error) { directoryCode = error.code; } var asyncFilterError; try { fs.cpSync(source + '/keep.txt', root + '/invalid-filter', { filter: function() { return Promise.resolve(true); } }); } catch (error) { asyncFilterError = error.name; } return [fs.existsSync(sync + '/keep.txt'), fs.existsSync(sync + '/skip.txt'), fs.readlinkSync(sync + '/link'), Math.abs(fs.statSync(sync + '/keep.txt').mtimeMs - 2000000) < 2, syncSeen.length, fs.readFileSync(promised + '/link', 'utf8'), fs.existsSync(promised + '/nested'), asyncSeen.length, fs.existsSync(callback + '/keep.txt'), fs.existsSync(callback + '/skip.txt'), callbackSeen.length, directoryCode, asyncFilterError]; };",
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
        r#"[true,false,"keep.txt",true,6,"keep",false,5,true,false,6,"ERR_FS_EISDIR","TypeError"]"#
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
        r#"[true,9,"abcde",["ab","cd","e"],true,true,-1,true,"EBADF","function"]"#
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
            "var promises = require('node:fs/promises'); module.exports = async function (path) { var handle = await promises.open(path, 'w+'); await handle.writeFile('abcdef'); var first = Buffer.alloc(4, 46), read = await handle.read(first, 1, 2, 2); var written = await handle.write(Buffer.from('XYZ'), 1, 2, 4); var textWrite = await handle.write('!', 1, 'utf8'); var sequential = Buffer.alloc(3), sequentialRead = await handle.read(sequential, 0, 3, null); var value = await handle.readFile('utf8'); await handle.close(); return [read.bytesRead, read.buffer.toString(), written.bytesWritten, written.buffer.toString(), textWrite.bytesWritten, textWrite.buffer, sequentialRead.bytesRead, sequential.toString(), value]; };",
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
    assert_eq!(result, r#"[2,".cd.",2,"XYZ",1,"!",3,"a!c","a!cdYZ"]"#);
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
fn fs_creation_apis_honor_requested_modes() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_fs_creation_modes");
    fs::write(
            dir.join("index.js"),
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (root) { var sync = root + '/sync.txt', callback = root + '/callback.txt', promised = root + '/promised.txt', opened = root + '/opened.txt', asyncOpened = root + '/async-opened.txt', streamed = root + '/streamed.txt', directory = root + '/directory'; fs.writeFileSync(sync, 'sync', { mode: 384 }); await new Promise(function(resolve, reject) { fs.appendFile(callback, 'callback', { mode: '600' }, function(error) { error ? reject(error) : resolve(); }); }); await promises.writeFile(promised, 'promised', { mode: 384 }); var fd = fs.openSync(opened, 'w', 384); fs.closeSync(fd); var handle = await promises.open(asyncOpened, 'w', 384); await handle.close(); fs.mkdirSync(directory, { mode: 448 }); var writer = fs.createWriteStream(streamed, { mode: 384 }); await new Promise(function(resolve, reject) { writer.on('error', reject); writer.on('finish', resolve); writer.end('stream'); }); return [sync, callback, promised, opened, asyncOpened, streamed, directory].map(function(path) { return fs.statSync(path).mode & 511; }); };",
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
    assert_eq!(result, r#"[384,384,384,384,384,384,448]"#);
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
            "var fs = require('node:fs'), promises = require('node:fs/promises'); module.exports = async function (path) { fs.writeFileSync(path, 'abcdef'); fs.truncateSync(path, 3); await new Promise(function(resolve, reject) { fs.truncate(path, 5, function(error) { error ? reject(error) : resolve(); }); }); await promises.truncate(path, 4); var handle = await promises.open(path, 'r+'); await handle.chmod(384); await handle.sync(); await handle.datasync(); await handle.close(); var closedCode; try { await handle.sync(); } catch (error) { closedCode = error.code; } return [fs.statSync(path).size, fs.readFileSync(path).toString('hex'), closedCode]; };",
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
    assert_eq!(result, r#"[4,"61626300","EBADF"]"#);
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

