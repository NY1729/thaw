#[test]
fn console_builtin_shares_global_console_and_constructor() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_console");
    fs::write(
            dir.join("index.js"),
            "var consoleModule = require('node:console');\n\
             module.exports = function () { var output = []; var instance = new consoleModule.Console({ write: function(value) { output.push(value); } }); instance.log('%s:%d', 'value', 2); instance.warn({ ok: true }); return [consoleModule === globalThis.console, consoleModule.console === globalThis.console, output.join('')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_console_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseConsole = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseConsole").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,true,"value:2\n{\"ok\":true}\n"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn crypto_builtin_shares_native_hash_and_random_implementations() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_crypto");
    fs::write(
            dir.join("index.js"),
            "var crypto = require('node:crypto');\n\
             module.exports = async function () { var callbackLength = await new Promise(function(resolve, reject) { crypto.randomBytes(7, function(error, value) { if (error) reject(error); else resolve(value.length); }); }); var uuid = crypto.randomUUID(); return [crypto === globalThis.__thaw_crypto_module, crypto.createHash('sha256').update('abc').digest('hex'), crypto.createHmac('sha512', 'key').update('value').digest().length, callbackLength, /^[0-9a-f-]{36}$/.test(uuid), crypto.webcrypto === globalThis.crypto, crypto.timingSafeEqual(Buffer.from('x'), Buffer.from('x'))]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_crypto_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseCrypto = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseCrypto").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",64,7,true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn perf_hooks_builtin_shares_the_performance_timeline() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_perf_hooks");
    fs::write(
            dir.join("index.js"),
            "var hooks = require('node:perf_hooks');\n\
             module.exports = function () { hooks.performance.clearMarks(); hooks.performance.clearMeasures(); hooks.performance.mark('start', { startTime: 2 }); hooks.performance.mark('end', { startTime: 7 }); var measure = hooks.performance.measure('elapsed', 'start', 'end'); var histogram = hooks.monitorEventLoopDelay({ resolution: 10 }); var enabled = histogram.enable(); var disabled = histogram.disable(); var utilization = hooks.performance.eventLoopUtilization(); return [hooks.performance === globalThis.performance, measure.duration, hooks.PerformanceObserver === globalThis.PerformanceObserver, enabled, disabled, histogram.percentile(99), typeof histogram.percentileBigInt(99), utilization.idle, utilization.active >= 0, utilization.utilization >= 0, hooks.performance.nodeTiming.name, hooks.constants.NODE_PERFORMANCE_GC_MAJOR]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_perf_hooks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exercisePerformanceHooks = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exercisePerformanceHooks").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,5,true,true,true,0,"bigint",0,true,true,"node",4]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn v8_builtin_serializes_graphs_and_exposes_runtime_statistics() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_v8");
    fs::write(
            dir.join("index.js"),
            "var v8 = require('node:v8');\n\
             module.exports = function () { var source = { bigint: 42n, bytes: Buffer.from('thaw'), map: new Map([['answer', 42]]), set: new Set(['x']), missing: undefined }; source.self = source; var encoded = v8.serialize(source); var copy = v8.deserialize(encoded); var heap = v8.getHeapStatistics(); var code = v8.getHeapCodeStatistics(); return [Buffer.isBuffer(encoded), copy !== source, copy.self === copy, copy.bigint === 42n, copy.bytes.toString(), copy.map.get('answer'), copy.set.has('x'), Object.prototype.hasOwnProperty.call(copy, 'missing'), copy.missing === undefined, heap.number_of_native_contexts, heap.heap_size_limit > 0, v8.getHeapSpaceStatistics().length, code.code_and_metadata_size, v8.cachedDataVersionTag()]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_v8_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseV8 = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseV8").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,true,true,"thaw",42,true,true,true,1,true,0,0,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn zlib_builtin_compresses_sync_and_callback_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_zlib");
    fs::write(dir.join("index.js"), "var zlib = require('node:zlib'), stream = require('node:stream'); module.exports = async function () { var source = Buffer.from('thaw compression '.repeat(8)); var gzip = zlib.gzipSync(source); var raw = zlib.deflateRawSync(source); var brotli = zlib.brotliCompressSync(source); var callback = await new Promise(function(resolve, reject) { zlib.gunzip(gzip, function(error, value) { error ? reject(error) : resolve(value); }); }); var brotliCallback = await new Promise(function(resolve, reject) { zlib.brotliDecompress(brotli, function(error, value) { error ? reject(error) : resolve(value); }); }); var compressor = zlib.createGzip(), compressed = []; compressor.on('data', function(value) { compressed.push(value); }); var streamDone = new Promise(function(resolve, reject) { compressor.on('error', reject); compressor.on('end', resolve); }); compressor.write(source.subarray(0, 7)); compressor.end(source.subarray(7)); await streamDone; var streamed = Buffer.concat(compressed), decompressor = zlib.createGunzip(), restored = []; decompressor.on('data', function(value) { restored.push(value); }); var restoreDone = new Promise(function(resolve, reject) { decompressor.on('error', reject); decompressor.on('end', resolve); }); stream.Readable.from([streamed.subarray(0, 5), streamed.subarray(5)]).pipe(decompressor); await restoreDone; var brotliStream = zlib.createBrotliCompress(), brotliChunks = []; brotliStream.on('data', function(value) { brotliChunks.push(value); }); var brotliDone = new Promise(function(resolve, reject) { brotliStream.on('error', reject); brotliStream.on('end', resolve); }); brotliStream.end(source); await brotliDone; var flushed = await new Promise(function(resolve) { compressor.flush(resolve); }); return [gzip[0], gzip[1], zlib.gunzipSync(gzip).toString() === source.toString(), zlib.inflateRawSync(raw).toString() === source.toString(), callback.toString() === source.toString(), zlib.constants.Z_OK, zlib.constants.Z_SYNC_FLUSH, compressor instanceof zlib.Gzip, compressor instanceof stream.Transform, compressor.bytesWritten, Buffer.concat(restored).toString() === source.toString(), flushed, zlib.brotliDecompressSync(brotli).toString() === source.toString(), brotliCallback.toString() === source.toString(), brotliStream instanceof zlib.BrotliCompress, zlib.brotliDecompressSync(Buffer.concat(brotliChunks)).toString() === source.toString(), zlib.constants.BROTLI_OPERATION_FINISH]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_zlib_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseZlib = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseZlib").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[31,139,true,true,true,0,2,true,true,136,true,null,true,true,true,true,2]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_threads_builtin_exchanges_cloned_messages() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_threads");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var channel = new workers.MessageChannel(); var original = { value: 7 }; channel.port1.postMessage(original); original.value = 9; var received = workers.receiveMessageOnPort(channel.port2); var pending = new Promise(function(resolve) { channel.port2.once('message', resolve); }); channel.port1.postMessage(new Map([['answer', 42]])); var asynchronous = await pending; workers.setEnvironmentData('config', { enabled: true }); var environment = workers.getEnvironmentData('config'); environment.enabled = false; var freshEnvironment = workers.getEnvironmentData('config'); var protectedBuffer = new ArrayBuffer(4); workers.markAsUntransferable(protectedBuffer); var transferRejected = false; try { channel.port1.postMessage('x', [protectedBuffer]); } catch (error) { transferRejected = error.name === 'DataCloneError'; } var protectedObject = { value: 1 }; workers.markAsUncloneable(protectedObject); var cloneRejected = false; try { structuredClone({ nested: protectedObject }); } catch (error) { cloneRejected = error.name === 'DataCloneError'; } var extra = new workers.MessageChannel(); channel.port1.postMessage({ port: extra.port1 }, [extra.port1]); var transferred = workers.receiveMessageOnPort(channel.port2); transferred.message.port.postMessage('through'); var through = workers.receiveMessageOnPort(extra.port2).message; var broadcastSender = new workers.BroadcastChannel('sync-receive'), broadcastReceiver = new workers.BroadcastChannel('sync-receive'); broadcastSender.postMessage({ value: 5 }); var broadcast = workers.receiveMessageOnPort(broadcastReceiver); broadcastSender.close(); broadcastReceiver.close(); extra.port1.close(); extra.port2.close(); channel.port1.unref(); var refed = channel.port1.hasRef(); channel.port1.ref(); channel.port1.close(); channel.port2.close(); return [workers.isMainThread, workers.threadId, workers.parentPort, received.message.value, asynchronous.get('answer'), freshEnvironment.enabled, refed, channel.port1.hasRef(), workers.SHARE_ENV === Symbol.for('nodejs.worker_threads.SHARE_ENV'), workers.receiveMessageOnPort(channel.port2) === undefined, workers.isMarkedAsUntransferable(protectedBuffer), transferRejected, cloneRejected, broadcast.message.value, through]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_worker_threads_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkers = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseWorkers").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,0,null,7,42,true,false,true,true,true,true,true,true,5,"through"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_threads_eval_worker_isolates_state_and_exchanges_messages() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_eval");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var events = []; delete process.env.THAW_WORKER_TEST; var source = \"var wt = require('node:worker_threads'); globalThis.workerOnly = 99; wt.parentPort.on('message', function(value) { var args = process.argv.slice(-2).join(','); var environment = process.env.THAW_WORKER_TEST; process.env.THAW_WORKER_TEST = 'mutated'; wt.parentPort.postMessage({ answer: wt.workerData.base + value, main: wt.isMainThread, threadId: wt.threadId, threadName: wt.threadName, isolated: globalThis.workerOnly, args: args, execArgv: process.execArgv.join(','), environment: environment, native: typeof __thaw_worker_spawn }); wt.parentPort.close(); });\"; var worker = new workers.Worker(source, { eval: true, name: 'alpha', workerData: { base: 40 }, argv: ['one', 2], execArgv: ['--trace-warnings'], env: { THAW_WORKER_TEST: 'child' }, resourceLimits: { maxOldGenerationSizeMb: 64 } }); var listenerOrder = []; function regular() { listenerOrder.push('regular'); } worker.addListener('probe', regular).prependOnceListener('probe', function() { listenerOrder.push('first'); }); var listenerCount = worker.listenerCount('probe'); var hasProbe = worker.eventNames().indexOf('probe') >= 0; worker.emit('probe'); worker.emit('probe'); worker.removeListener('probe', regular).setMaxListeners(20); var completed = new Promise(function(resolve) { worker.on('online', function() { events.push('online'); }); worker.on('message', function(value) { events.push('message:' + value.answer + ':' + value.main + ':' + (value.threadId > 0) + ':' + value.threadName + ':' + value.isolated + ':' + value.args + ':' + value.execArgv + ':' + value.environment + ':' + value.native); }); worker.on('error', function(error) { events.push('error:' + error.stack); resolve(); }); worker.on('exit', function(code) { events.push('exit:' + code); resolve(); }); }); worker.postMessage(2); await completed; delete process.env.THAW_WORKER_SHARED; var shared = new workers.Worker(\"var wt = require('node:worker_threads'); process.env.THAW_WORKER_SHARED = 'shared'; wt.parentPort.close();\", { eval: true, env: workers.SHARE_ENV }); await new Promise(function(resolve, reject) { shared.on('error', reject); shared.on('exit', resolve); }); var sharedValue = process.env.THAW_WORKER_SHARED; delete process.env.THAW_WORKER_SHARED; return [events, worker.threadName, listenerOrder, listenerCount, hasProbe, worker.listenerCount('probe'), worker.getMaxListeners(), typeof globalThis.workerOnly, process.env.THAW_WORKER_TEST, sharedValue, worker.resourceLimits.maxOldGenerationSizeMb, worker.threadId > 0, worker.ref() === worker, worker.unref() === worker, await worker.terminate()]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_worker_eval_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorker = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseWorker").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["online","message:42:false:true:alpha:99:one,2:--trace-warnings:child:function","exit:0"],"alpha",["first","regular","regular"],2,true,0,20,"undefined",null,"shared",64,true,true,true,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_threads_share_environment_across_native_workers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_native_share_env");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); } function result(worker) { return new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); }); } module.exports = async function () { delete process.env.THAW_PARENT_SHARED; delete process.env.THAW_WORKER_SHARED; delete process.env.THAW_SECOND_SHARED; var first = new workers.Worker(\"var wt = require('node:worker_threads'); wt.parentPort.on('message', function() { var parent = process.env.THAW_PARENT_SHARED; process.env.THAW_WORKER_SHARED = 'worker'; wt.parentPort.postMessage(parent); wt.parentPort.close(); });\", { eval: true, env: workers.SHARE_ENV }); await online(first); process.env.THAW_PARENT_SHARED = 'parent'; var firstResult = result(first), firstExit = new Promise(function(resolve) { first.on('exit', resolve); }); first.postMessage('go'); var parentSeen = await firstResult; await firstExit; var second = new workers.Worker(\"var wt = require('node:worker_threads'); wt.parentPort.postMessage(process.env.THAW_WORKER_SHARED); process.env.THAW_SECOND_SHARED = 'second'; wt.parentPort.close();\", { eval: true, env: workers.SHARE_ENV }); var secondResult = result(second), secondExit = new Promise(function(resolve) { second.on('exit', resolve); }); var workerSeen = await secondResult; await secondExit; var secondSeen = process.env.THAW_SECOND_SHARED, native = [typeof first._nativeHandle, typeof second._nativeHandle]; delete process.env.THAW_PARENT_SHARED; delete process.env.THAW_WORKER_SHARED; delete process.env.THAW_SECOND_SHARED; return [parentSeen, workerSeen, secondSeen, native]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_native_share_env_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeShareEnv = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseNativeShareEnv").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["parent","worker","second",["number","number"]]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_moves_message_ports_to_vm_contexts() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_move_port_context");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); var vm = require('node:vm'); module.exports = function () { var channel = new workers.MessageChannel(); var context = vm.createContext({}); var moved = workers.moveMessagePortToContext(channel.port1, context); moved.postMessage('moved'); var received = workers.receiveMessageOnPort(channel.port2); var invalidContext = false; try { workers.moveMessagePortToContext(channel.port2, {}); } catch (error) { invalidContext = error instanceof TypeError; } return [received.message, channel.port1.__thawClosed, moved !== channel.port1, invalidContext]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_move_port_context_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseMovePortToContext = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseMovePortToContext").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(result, r#"["moved",true,true,true]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_transfers_worker_data_message_ports() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_data_transfer");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var channel = new workers.MessageChannel(); var source = \"var wt = require('node:worker_threads'); wt.workerData.port.postMessage('from-worker'); wt.workerData.port.on('message', function(value) { wt.workerData.port.postMessage(value.toUpperCase()); wt.workerData.port.close(); wt.parentPort.close(); });\"; var worker = new workers.Worker(source, { eval: true, workerData: { port: channel.port1 }, transferList: [channel.port1] }); var first = new Promise(function(resolve) { channel.port2.once('message', resolve); }); var detached = channel.port1.__thawClosed && channel.port1.__thawPeer === null; var exit = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); var initial = await first, second = new Promise(function(resolve) { channel.port2.once('message', resolve); }); channel.port2.postMessage('through'); return [initial, await second, await exit, detached, typeof worker._nativeHandle]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_data_transfer_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkerDataTransfer = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseWorkerDataTransfer").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(result, r#"["from-worker","THROUGH",0,true,"number"]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_transfer_message_ports_from_native_workers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_native_outbound_port");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = \"var wt = require('node:worker_threads'), channel = new wt.MessageChannel(); channel.port2.on('message', function(value) { channel.port2.postMessage(value.toUpperCase()); channel.port2.close(); wt.parentPort.close(); }); wt.parentPort.postMessage({ port: channel.port1 }, [channel.port1]);\"; var worker = new Worker(source, { eval: true }); var exit = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); var transferred = await new Promise(function(resolve) { worker.once('message', function(value) { resolve(value.port); }); }); var response = new Promise(function(resolve) { transferred.once('message', resolve); }); transferred.postMessage('worker-port'); var value = await response, code = await exit; transferred.close(); return [value, code, typeof worker._nativeHandle, transferred.__thawHostPortId.startsWith('w:')]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_native_outbound_port_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeOutboundPort = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseNativeOutboundPort").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(result, r#"["WORKER-PORT",0,"number",true]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_redirects_stdin_stdout_and_stderr() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_stdio");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = \"var wt = require('node:worker_threads'); process.stdin.setEncoding('utf8'); process.stdin.on('data', function(value) { console.log('stdout', value); console.error('stderr', value); wt.parentPort.postMessage(value.toUpperCase()); wt.parentPort.close(); });\"; var worker = new Worker(source, { eval: true, stdin: true, stdout: true, stderr: true }); worker.stdout.setEncoding('utf8'); worker.stderr.setEncoding('utf8'); var output = '', errors = '', message; worker.stdout.on('data', function(value) { output += value; }); worker.stderr.on('data', function(value) { errors += value; }); worker.on('message', function(value) { message = value; }); var exited = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); worker.stdin.end('hello'); var code = await exited; return [message, output, errors, code, worker.stdin.writableFinished, worker.stdout.readableEnded, worker.stderr.readableEnded, typeof worker._nativeHandle]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_stdio_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkerStdio = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseWorkerStdio").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["HELLO","stdout hello\n","stderr hello\n",0,true,true,true,"number"]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_terminate_resolves_with_one_shared_exit_code() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_terminate");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var worker = new Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); var exits = []; worker.on('exit', function(code) { exits.push(code); }); var first = worker.terminate(); var second = worker.terminate(); var code = await first; return [first === second, code, await second, await worker.terminate(), exits, Symbol.asyncDispose ? worker[Symbol.asyncDispose] === worker.terminate : true]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_terminate_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkerTerminate = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseWorkerTerminate").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(result, "[true,1,1,1,[1],true]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_diagnostics_follow_running_state() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_diagnostics");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var worker = new Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); await new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); var cpu = await worker.cpuUsage(); var heap = await worker.getHeapStatistics(); var snapshot = await worker.getHeapSnapshot(); snapshot.setEncoding('utf8'); var text = ''; await new Promise(function(resolve, reject) { snapshot.on('data', function(chunk) { text += chunk; }); snapshot.on('end', resolve); snapshot.on('error', reject); }); var profile = await worker.startCpuProfile(); var stopped = await profile.stop(); await worker.terminate(); var stoppedError; try { await worker.cpuUsage(); } catch (error) { stoppedError = error.code; } return [cpu, heap.number_of_native_contexts, JSON.parse(text).snapshot.node_count, stopped.nodes.length, stopped.samples.length, stoppedError]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_diagnostics_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseWorkerDiagnostics = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseWorkerDiagnostics").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[{"user":0,"system":0},1,0,0,0,"ERR_WORKER_NOT_RUNNING"]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_routes_messages_by_thread_id() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_direct_messages");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); } module.exports = async function () { var fromWorker = []; function mainListener(value, source) { fromWorker.push([value.kind, source]); } process.on('workerMessage', mainListener); var source = \"var wt = require('node:worker_threads'); process.on('workerMessage', function(value, source) { wt.parentPort.postMessage(['direct:' + value.kind, source]); wt.parentPort.close(); }); wt.postMessageToThread(0, { kind: 'hello' }).then(function() { wt.parentPort.postMessage(['ready', wt.threadId]); });\"; var worker = new workers.Worker(source, { eval: true }); var messages = []; var direct; var completed = new Promise(function(resolve, reject) { worker.on('message', function(value) { messages.push(value); if (value[0] === 'ready') workers.postMessageToThread(worker.threadId, { kind: 'ping' }).catch(reject); else direct = value; }); worker.on('error', reject); worker.on('exit', resolve); }); var exitCode = await completed; process.off('workerMessage', mainListener); var same, missing; try { await workers.postMessageToThread(0, 'x'); } catch (error) { same = error.code; } try { await workers.postMessageToThread(999999, 'x'); } catch (error) { missing = error.code; } var silent = new workers.Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); await online(silent); var noListener; try { await workers.postMessageToThread(silent.threadId, 'x'); } catch (error) { noListener = error.code; } await silent.terminate(); var throwing = new workers.Worker(\"var wt = require('node:worker_threads'); process.on('workerMessage', function() { throw new Error('listener failed'); });\", { eval: true }); await online(throwing); var listenerError; try { await workers.postMessageToThread(throwing.threadId, 'x'); } catch (error) { listenerError = [error.code, error.cause.message]; } var native = [typeof worker._nativeHandle, typeof silent._nativeHandle, typeof throwing._nativeHandle]; await throwing.terminate(); return [fromWorker, direct, exitCode, same, missing, noListener, listenerError, native]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_direct_messages_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDirectWorkerMessages = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseDirectWorkerMessages").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[["hello",1]],["direct:ping",0],0,"ERR_WORKER_MESSAGING_SAME_THREAD","ERR_WORKER_MESSAGING_FAILED","ERR_WORKER_MESSAGING_FAILED",["ERR_WORKER_MESSAGING_ERRORED","listener failed"],["number","number","number"]]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_route_direct_messages_between_native_workers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_direct_between_workers");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); } module.exports = async function () { var receiver = new workers.Worker(\"var wt = require('node:worker_threads'); process.on('workerMessage', function(value, source) { wt.parentPort.postMessage([value.answer, source]); wt.parentPort.close(); });\", { eval: true }); await online(receiver); var sender = new workers.Worker(\"var wt = require('node:worker_threads'); wt.postMessageToThread(wt.workerData.target, { answer: 42 }).then(function() { wt.parentPort.postMessage('sent'); wt.parentPort.close(); });\", { eval: true, workerData: { target: receiver.threadId } }); var received = new Promise(function(resolve, reject) { receiver.on('message', resolve); receiver.on('error', reject); }); var sent = new Promise(function(resolve, reject) { sender.on('message', resolve); sender.on('error', reject); }); var receiverExit = new Promise(function(resolve) { receiver.on('exit', resolve); }), senderExit = new Promise(function(resolve) { sender.on('exit', resolve); }); var values = await Promise.all([received, sent, receiverExit, senderExit]); return [values, typeof receiver._nativeHandle, typeof sender._nativeHandle]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_direct_between_workers_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeWorkerRouting = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseNativeWorkerRouting").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(result, r#"[[[42,2],"sent",0,0],"number","number"]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_data_url_worker_decodes_javascript() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_data_url");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var parentThread = globalThis.__thaw_os_thread_token; var source = \"var wt = require('node:worker_threads'); wt.parentPort.postMessage({ value: wt.workerData.value, main: wt.isMainThread, native: typeof __thaw_worker_spawn, thread: globalThis.__thaw_os_thread_token }); wt.parentPort.close();\"; var worker = new workers.Worker('data:text/javascript,' + encodeURIComponent(source), { workerData: { value: 17 } }); var events = []; await new Promise(function(resolve, reject) { worker.on('online', function() { events.push('online'); }); worker.on('message', function(value) { events.push('message:' + value.value + ':' + value.main + ':' + value.native + ':' + (value.thread !== parentThread)); }); worker.on('error', reject); worker.on('exit', function(code) { events.push('exit:' + code); resolve(); }); }); var rejected = false; try { new workers.Worker('data:text/plain,not-javascript'); } catch (error) { rejected = error instanceof TypeError; } return [events, rejected]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_worker_data_url_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDataWorker = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseDataWorker").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["online","message:17:false:function:true","exit:0"],true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_threads_load_runtime_computed_absolute_paths() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_runtime_path");
    let worker_dir = dir.join("workers/nested");
    fs::create_dir_all(&worker_dir).unwrap();
    fs::create_dir_all(dir.join("internal/features")).unwrap();
    fs::write(
            dir.join("package.json"),
            r##"{"imports":{"#worker-tool":{"node":"./internal/tool.cjs","default":"./internal/wrong.cjs"},"#features/*":{"require":"./internal/features/*.js"},"#escape":"../outside.js"}}"##,
        )
        .unwrap();
    fs::write(dir.join("internal/tool.cjs"), "module.exports = 11;").unwrap();
    fs::write(
        dir.join("internal/features/math.js"),
        "module.exports = 12;",
    )
    .unwrap();
    let worker_path = worker_dir.join("dynamic-worker.js");
    fs::write(
        worker_dir.join("worker-value.js"),
        "module.exports = require('./worker-package') + 1;",
    )
    .unwrap();
    fs::create_dir_all(worker_dir.join("worker-package/lib")).unwrap();
    fs::write(
        worker_dir.join("worker-package/package.json"),
        r#"{"main":"lib/value"}"#,
    )
    .unwrap();
    fs::write(
        worker_dir.join("worker-package/lib/value.cjs"),
        "module.exports = require('../worker-data.json').value;",
    )
    .unwrap();
    fs::write(
        worker_dir.join("worker-package/worker-data.json"),
        r#"{"value":41}"#,
    )
    .unwrap();
    fs::create_dir_all(dir.join("node_modules/runtime-worker-dependency")).unwrap();
    fs::write(
            dir.join("node_modules/runtime-worker-dependency/package.json"),
            r#"{"main":"wrong.cjs","exports":{".":{"node":"./entry.cjs","default":"./wrong.cjs"},"./feature":{"require":"./feature.js"},"./features/*":{"require":"./dist/*.cjs"},"./escape":"../outside.js"}}"#,
        )
        .unwrap();
    fs::write(
        dir.join("node_modules/runtime-worker-dependency/entry.cjs"),
        "module.exports = 8;",
    )
    .unwrap();
    fs::write(
        dir.join("node_modules/runtime-worker-dependency/feature.js"),
        "module.exports = 9;",
    )
    .unwrap();
    fs::create_dir_all(dir.join("node_modules/runtime-worker-dependency/dist")).unwrap();
    fs::write(
        dir.join("node_modules/runtime-worker-dependency/dist/math.cjs"),
        "module.exports = 10;",
    )
    .unwrap();
    fs::write(
            &worker_path,
            "var wt = require('node:worker_threads'); var hiddenCode; try { require('runtime-worker-dependency/hidden'); } catch (error) { hiddenCode = error.code; } var missingImportCode; try { require('#missing'); } catch (error) { missingImportCode = error.code; } var exportEscapeCode; try { require('runtime-worker-dependency/escape'); } catch (error) { exportEscapeCode = error.code; } var importEscapeCode; try { require('#escape'); } catch (error) { importEscapeCode = error.code; } wt.parentPort.postMessage([wt.workerData, __filename, __dirname, typeof __thaw_worker_spawn, require('./worker-value'), require('runtime-worker-dependency'), require('runtime-worker-dependency/feature'), require('runtime-worker-dependency/features/math'), hiddenCode, require('#worker-tool'), require('#features/math'), missingImportCode, exportEscapeCode, importEscapeCode]); wt.parentPort.close();",
        )
        .unwrap();
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function (runtimePath) { var worker = new Worker(runtimePath, { workerData: 23 }); var exit = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); var message = await new Promise(function(resolve) { worker.on('message', resolve); }); return [message, await exit, typeof worker._nativeHandle]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_runtime_path_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseRuntimeWorkerPath = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseRuntimeWorkerPath").unwrap();
    let path = worker_path.to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&vec![&path]).unwrap()).unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    let expected = serde_json::json!([
        [
            23,
            path,
            worker_dir.to_string_lossy(),
            "function",
            42,
            8,
            9,
            10,
            "ERR_PACKAGE_PATH_NOT_EXPORTED",
            11,
            12,
            "ERR_PACKAGE_IMPORT_NOT_DEFINED",
            "ERR_INVALID_PACKAGE_TARGET",
            "ERR_INVALID_PACKAGE_TARGET"
        ],
        0,
        "number"
    ])
    .to_string();
    assert_eq!(result, expected);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_runtime_round_trips_parent_messages() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_native_round_trip");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = \"var wt = require('node:worker_threads'); wt.parentPort.on('message', function(value) { wt.parentPort.postMessage([value * wt.workerData, globalThis.__thaw_os_thread_token]); wt.parentPort.close(); });\"; var parentThread = globalThis.__thaw_os_thread_token; var worker = new Worker(source, { eval: true, workerData: 7 }); var exit = new Promise(function(resolve) { worker.on('exit', resolve); }); var result = await new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); worker.postMessage(6); }); var code = await exit; return [result[0], result[1] !== parentThread, code, typeof worker._nativeHandle]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_native_round_trip_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeWorkerRoundTrip = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseNativeWorkerRoundTrip").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(result, r#"[42,true,0,"number"]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_runtime_transfers_structured_array_buffers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_native_structured_transfer");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var first = new ArrayBuffer(3), second = new ArrayBuffer(2); new Uint8Array(first).set([1, 2, 3]); new Uint8Array(second).set([4, 5]); var data = { buffer: first, map: new Map([['answer', 42]]) }; data.self = data; var source = \"var wt = require('node:worker_threads'); wt.parentPort.on('message', function(value) { var reply = { first: Array.from(new Uint8Array(wt.workerData.buffer)), second: Array.from(new Uint8Array(value.buffer)), answer: wt.workerData.map.get('answer'), workerCycle: wt.workerData.self === wt.workerData, messageCycle: value.self === value }; reply.self = reply; wt.parentPort.postMessage(reply); wt.parentPort.close(); });\"; var worker = new Worker(source, { eval: true, workerData: data, transferList: [first] }); var message = { buffer: second }; message.self = message; var exit = new Promise(function(resolve) { worker.on('exit', resolve); }); var reply = new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); }); worker.postMessage(message, [second]); var value = await reply, code = await exit; return [first.byteLength, second.byteLength, value.first, value.second, value.answer, value.workerCycle, value.messageCycle, value.self === value, code, typeof worker._nativeHandle]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_native_structured_transfer_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNativeWorkerTransfer = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let function = CString::new("exerciseNativeWorkerTransfer").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[0,0,[1,2,3],[4,5],42,true,true,true,0,"number"]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn bundler_embeds_static_file_url_worker_sources() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_worker_file_url");
    fs::write(
        dir.join("double.js"),
        "module.exports = function(value) { return value * 2; };",
    )
    .unwrap();
    fs::write(
            dir.join("worker.js"),
            "import { parentPort, workerData } from 'node:worker_threads'; var double = require('./double'); parentPort.postMessage(double(workerData)); parentPort.close();",
        )
        .unwrap();
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; var path = require('node:path'); const workerFile = './' + 'worker.js'; const joinedWorkerFile = path.join(__dirname, 'worker.js'); function run(worker, events) { return new Promise(function(resolve, reject) { worker.on('message', function(value) { events.push(value); }); worker.on('error', reject); worker.on('exit', function(code) { events.push(code); resolve(); }); }); } module.exports = async function () { var events = []; await run(new Worker(new URL('./worker.js', import.meta.url), { workerData: 21 }), events); await run(new Worker('./worker.js', { workerData: 11 }), events); await run(new Worker(`./${'worker'}.js`, { workerData: 5 }), events); await run(new Worker(workerFile, { workerData: 3 }), events); await run(new Worker(joinedWorkerFile, { workerData: 2 }), events); return events; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_worker_file_url_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 6);
    fs::remove_dir_all(&dir).unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFileWorker = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseFileWorker").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "[42,0,22,0,10,0,6,0,4,0]");
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_url_rewrite_ignores_unrelated_worker_bindings() {
    let dir = temp_registry("unrelated_worker_url");
    let source = "class Worker {}\nnew Worker(new URL('./missing.js', import.meta.url));";
    let rewritten = rewrite_static_worker_urls(source, &dir.join("index.js"), "pkg", &dir)
        .expect("an unrelated Worker must not attempt to read its URL");
    assert_eq!(rewritten, source);
    let eval_source = "var Worker = require('node:worker_threads').Worker; new Worker('./missing.js', { eval: true });";
    let rewritten = rewrite_static_worker_urls(eval_source, &dir.join("index.js"), "pkg", &dir)
        .expect("eval Worker source must not be treated as a file");
    assert_eq!(rewritten, eval_source);
    let shadowed_source = "var Worker = require('node:worker_threads').Worker; const workerFile = './missing.js'; function start(workerFile) { return new Worker(workerFile); }";
    let rewritten = rewrite_static_worker_urls(shadowed_source, &dir.join("index.js"), "pkg", &dir)
        .expect("a shadowed constant Worker path must remain dynamic");
    assert_eq!(rewritten, shadowed_source);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn worker_threads_broadcast_channel_clones_between_matching_names() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_broadcast_channel");
    fs::write(
            dir.join("index.js"),
            "var BroadcastChannel = require('node:worker_threads').BroadcastChannel; module.exports = async function () { var sender = new BroadcastChannel('room'); var first = new BroadcastChannel('room'); var second = new BroadcastChannel('room'); var isolated = new BroadcastChannel('elsewhere'); var source = { nested: { value: 4 } }; var firstMessage = new Promise(function(resolve) { first.onmessage = function(event) { event.data.nested.value = 8; resolve(event.data.nested.value); }; }); var secondMessage = new Promise(function(resolve) { second.addEventListener('message', function(event) { resolve(event.data.nested.value); }, { once: true }); }); var isolatedCalled = false; isolated.onmessage = function() { isolatedCalled = true; }; sender.postMessage(source); source.nested.value = 9; var values = await Promise.all([firstMessage, secondMessage]); first.close(); var closedError = false; try { first.postMessage('x'); } catch (error) { closedError = error.name === 'InvalidStateError'; } sender.close(); second.close(); isolated.close(); return [values[0], values[1], isolatedCalled, closedError, sender.name, sender.ref() === sender, sender.unref() === sender]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_broadcast_channel_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseBroadcast = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseBroadcast").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[8,4,false,true,"room",true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

