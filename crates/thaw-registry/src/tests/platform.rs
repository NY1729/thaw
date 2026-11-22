#[test]
fn console_builtin_shares_global_console_and_constructor() {
    let dir = temp_registry("builtin_console");
    fs::write(
        dir.join("index.js"),
        "var consoleModule = require('node:console');\n\
         module.exports = function () { var output = []; var Writable = require('node:stream').Writable; var sink = new Writable({ write: function(chunk, encoding, callback) { output.push(chunk.toString()); callback(); } }); var instance = new consoleModule.Console({ stdout: sink, stderr: sink }); instance.log('%s:%d', 'value', 2); instance.warn({ ok: true }); return [consoleModule === globalThis.console, output.join('')]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_console_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    assert_eq!(
        run_js(&bundle),
        r#"[true,"value:2\n{ ok: true }\n"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn crypto_builtin_shares_native_hash_and_random_implementations() {
    
    let dir = temp_registry("builtin_crypto");
    fs::write(
            dir.join("index.js"),
            "var crypto = require('node:crypto');\n\
             module.exports = async function () { var callbackLength = await new Promise(function(resolve, reject) { crypto.randomBytes(7, function(error, value) { if (error) reject(error); else resolve(value.length); }); }); var uuid = crypto.randomUUID(); return [crypto === globalThis.__thaw_crypto_module, crypto.createHash('sha256').update('abc').digest('hex'), crypto.createHash('sha1').update('abc').digest('base64'), crypto.createHmac('sha512', 'key').update('value').digest().length, callbackLength, /^[0-9a-f-]{36}$/.test(uuid), crypto.webcrypto === globalThis.crypto, crypto.timingSafeEqual(Buffer.from('x'), Buffer.from('x')), typeof crypto.subtle === 'object' && crypto.subtle === crypto.webcrypto.subtle]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_crypto_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let result = run_js(&bundle);
    assert_eq!(
        result,
        r#"[true,"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad","qZk+NkcGgWq6PiVxeFDCbJzQ2J0=",64,7,true,true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn perf_hooks_builtin_shares_the_performance_timeline() {
    
    let dir = temp_registry("builtin_perf_hooks");
    fs::write(
            dir.join("index.js"),
            "var originalHrtimeBigint = process.hrtime.bigint, clockValues = [10n, 20n, 25n, 25n, 30n, 100n, 104n], clockIndex = 0; process.hrtime.bigint = function() { if (clockIndex >= clockValues.length) throw new Error('fixture clock exhausted'); return clockValues[clockIndex++]; }; var hooks; try { hooks = require('node:perf_hooks'); } finally { process.hrtime.bigint = originalHrtimeBigint; }\n\
             module.exports = function () { hooks.performance.clearMarks(); hooks.performance.clearMeasures(); hooks.performance.mark('start', { startTime: 2 }); hooks.performance.mark('end', { startTime: 7 }); var measure = hooks.performance.measure('elapsed', 'start', 'end'); var originalTimeoutRef = globalThis.__thaw_set_timeout_ref, scheduled = []; var owned; try { globalThis.__thaw_set_timeout_ref = function(callback, delay, refed) { var handle = originalTimeoutRef(callback, delay, refed); scheduled.push({ callback: callback, handle: handle, refed: refed }); return handle; }; owned = hooks.monitorEventLoopDelay({ resolution: 10 }); if (owned.enable() !== true || owned._timer.hasRef() !== false) throw new Error('initial monitor timer must be unreferenced'); if (owned.enable() !== false) throw new Error('redundant enable must return false'); var first = scheduled[0]; if (!first || first.refed !== false) throw new Error('initial bridge call must pass false'); clearTimeout(first.handle); first.callback(); var second = scheduled[1]; if (!second || second.handle === first.handle || second.refed !== false || second.handle.hasRef() !== false) throw new Error('sample timer must be a distinct unreferenced real handle'); if (owned.disable() !== true || owned.disable() !== false) throw new Error('disable transitions must be true then false'); var savedCallback = second.callback, scheduledAfterDisable = scheduled.length; savedCallback(); if (scheduled.length !== scheduledAfterDisable) throw new Error('disabled callback must not reschedule'); } finally { try { if (owned) owned.disable(); } finally { try { scheduled.forEach(function(entry) { clearTimeout(entry.handle); }); } finally { globalThis.__thaw_set_timeout_ref = originalTimeoutRef; } } } function expectCode(action, name, code) { var caught; try { action(); } catch (error) { caught = error; } if (!caught || caught.name !== name || caught.code !== code) throw new Error('expected ' + name + ' ' + code); } var recordable = hooks.createHistogram(), monitor = hooks.monitorEventLoopDelay({ resolution: 10 }); if (typeof recordable.record !== 'function' || typeof recordable.recordDelta !== 'function' || typeof recordable.enable !== 'undefined' || typeof monitor.record !== 'undefined' || typeof monitor.recordDelta !== 'undefined') throw new Error('recordable and monitoring histogram methods must remain separate'); if (recordable.record(5) !== undefined || recordable.count !== 1 || recordable.min !== 5 || recordable.max !== 5) throw new Error('number record must return undefined and update statistics'); if (recordable.record(9007199254740993n) !== undefined || recordable.count !== 2 || !(recordable.max > 9007199254740991)) throw new Error('large bigint record must be accepted'); if (recordable.record(9223372036854775807n) !== undefined || recordable.count !== 3 || !(recordable.max > 9e18)) throw new Error('maximum int64 bigint record must be accepted'); var beforeCount = recordable.count, beforeMin = recordable.min, beforeMax = recordable.max, beforeMean = recordable.mean, beforeStddev = recordable.stddev; function unchanged() { if (recordable.count !== beforeCount || recordable.min !== beforeMin || recordable.max !== beforeMax || recordable.mean !== beforeMean || recordable.stddev !== beforeStddev) throw new Error('failed record must not mutate statistics'); } expectCode(function() { recordable.record('5'); }, 'TypeError', 'ERR_INVALID_ARG_TYPE'); unchanged(); expectCode(function() { recordable.record(0); }, 'RangeError', 'ERR_OUT_OF_RANGE'); unchanged(); expectCode(function() { recordable.record(1.5); }, 'RangeError', 'ERR_OUT_OF_RANGE'); unchanged(); expectCode(function() { recordable.record(9007199254740992); }, 'RangeError', 'ERR_OUT_OF_RANGE'); unchanged(); expectCode(function() { recordable.record(Infinity); }, 'RangeError', 'ERR_OUT_OF_RANGE'); unchanged(); expectCode(function() { recordable.record(9223372036854775808n); }, 'RangeError', 'ERR_OUT_OF_RANGE'); unchanged(); expectCode(function() { recordable.record.call({}, 1); }, 'TypeError', 'ERR_INVALID_THIS'); unchanged(); expectCode(function() { recordable.recordDelta.call({}); }, 'TypeError', 'ERR_INVALID_THIS'); unchanged(); var deltaHistogram = hooks.createHistogram(); if (deltaHistogram.recordDelta() !== undefined || deltaHistogram.count !== 0) throw new Error('first delta call must only set the baseline'); if (deltaHistogram.recordDelta() !== undefined || deltaHistogram.count !== 1 || deltaHistogram.min !== 10) throw new Error('second delta call must record the nanosecond difference'); deltaHistogram.record(4); if (deltaHistogram.recordDelta() !== undefined || deltaHistogram.count !== 3 || deltaHistogram.min !== 4 || deltaHistogram.max !== 10) throw new Error('public record must not change the delta baseline'); if (deltaHistogram.recordDelta() !== undefined || deltaHistogram.count !== 4 || deltaHistogram.min !== 0) throw new Error('zero delta must be recorded internally'); expectCode(function() { deltaHistogram.record(0); }, 'RangeError', 'ERR_OUT_OF_RANGE'); if (deltaHistogram.count !== 4 || deltaHistogram.min !== 0) throw new Error('failed record must leave statistics and delta baseline intact'); if (deltaHistogram.recordDelta() !== undefined || deltaHistogram.count !== 5 || deltaHistogram.max !== 10) throw new Error('failed record must not move the delta baseline'); deltaHistogram.reset(); if (deltaHistogram.count !== 0 || deltaHistogram.min !== 9223372036854775807 || deltaHistogram.max !== 0) throw new Error('reset must clear record statistics'); if (deltaHistogram.recordDelta() !== undefined || deltaHistogram.count !== 0) throw new Error('reset must reinitialize the delta baseline'); if (deltaHistogram.recordDelta() !== undefined || deltaHistogram.count !== 1 || deltaHistogram.min !== 4 || deltaHistogram.max !== 4) throw new Error('post-reset delta must start from the new baseline'); var histogram = hooks.monitorEventLoopDelay({ resolution: 10 }); var enabled = histogram.enable(); var disabled = histogram.disable(); var utilization = hooks.performance.eventLoopUtilization(); return [hooks.performance === globalThis.performance, measure.duration, hooks.PerformanceObserver === globalThis.PerformanceObserver, enabled, disabled, histogram.percentile(99), typeof histogram.percentileBigInt(99), utilization.idle, utilization.active >= 0, utilization.utilization >= 0, hooks.performance.nodeTiming.name, hooks.constants.NODE_PERFORMANCE_GC_MAJOR]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_perf_hooks_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let result = run_js(&bundle);
    assert_eq!(
        result,
        r#"[true,5,true,true,true,0,"bigint",0,true,true,"node",4]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

// v8.getHeapSpaceStatistics() (Node 22.23.1: 11 spaces in this order, one key set; does_zap_garbage is a boolean)
#[test]
fn v8_heap_space_statistics_lists_node_spaces_in_order() {
    let dir = temp_registry("builtin_v8_spaces");
    fs::write(
        dir.join("index.js"),
        "var v8 = require('node:v8'); module.exports = function () { var spaces = v8.getHeapSpaceStatistics(); return [spaces.length, spaces.map(function(space) { return space.space_name; }).join(','), Object.keys(spaces[0]).join(','), spaces.every(function(space) { return Object.keys(space).join() === Object.keys(spaces[0]).join(); }), v8.getHeapStatistics().does_zap_garbage]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_v8_spaces_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    // index.js plus the node:stream and node:fs modules the v8 module now requires (snapshot writer)
    assert_eq!(file_count, 5);
    assert_eq!(
        run_js(&bundle),
        r#"[11,"read_only_space,new_space,old_space,code_space,shared_space,trusted_space,new_large_object_space,large_object_space,code_large_object_space,shared_large_object_space,trusted_large_object_space","space_name,space_size,space_used_size,space_available_size,physical_space_size",true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

// v8.cachedDataVersionTag() (Node 22.23.1 returns a nonzero uint32, 2596627580 here; thaw's number differs by design)
#[test]
fn v8_cached_data_version_tag_is_a_stable_nonzero_u32() {
    let dir = temp_registry("builtin_v8_tag");
    fs::write(
        dir.join("index.js"),
        "var v8 = require('node:v8'); module.exports = function () { var tag = v8.cachedDataVersionTag(); return [Number.isInteger(tag), tag > 0, tag <= 0xFFFFFFFF, tag === v8.cachedDataVersionTag()]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_v8_tag_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    // index.js plus the node:stream and node:fs modules the v8 module now requires (snapshot writer)
    assert_eq!(file_count, 5);
    assert_eq!(run_js(&bundle), "[true,true,true,true]");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

// v8.getHeapSnapshot() (Node 22.23.1, 3 runs: 8 top-level keys; snapshot.meta is the literal below; node_count 43.3k-43.7k
// (thaw's registry graph is much smaller by design, D3, so only > 0 and the array-length invariants are asserted); the first
// node is the synthetic root (type 9, name index 1 = "", id 1); a setEncoding('utf8') stream yields strings, default yields Buffers)
#[test]
fn v8_get_heap_snapshot_is_valid_json_with_nodes_meta() {
    let dir = temp_registry("builtin_v8_snapshot");
    fs::write(
        dir.join("index.js"),
        r##"var v8 = require('node:v8');
var NODE_META = '{"node_fields":["type","name","id","self_size","edge_count","trace_node_id","detachedness"],"node_types":[["hidden","array","string","object","code","closure","regexp","number","native","synthetic","concatenated string","sliced string","symbol","bigint","object shape"],"string","number","number","number","number","number"],"edge_fields":["type","name_or_index","to_node"],"edge_types":[["context","element","property","internal","hidden","shortcut","weak"],"string_or_number","node"],"trace_function_info_fields":["function_id","name","script_name","script_id","line","column"],"trace_node_fields":["id","function_info_index","count","size","children"],"sample_fields":["timestamp_us","last_assigned_id"],"location_fields":["object_index","script_id","line","column"]}';
module.exports = async function () {
  var stream = v8.getHeapSnapshot();
  stream.setEncoding('utf8');
  var text = '', kinds = {};
  for await (var chunk of stream) { kinds[typeof chunk] = true; text += chunk; }
  var snap = JSON.parse(text), s = snap.snapshot, strings = snap.strings;
  var nodeNamesOk = snap.nodes.every(function (value, i) { return i % 7 !== 1 || (Number.isInteger(value) && value >= 0 && value < strings.length); });
  var nodeTypesOk = snap.nodes.every(function (value, i) { return i % 7 !== 0 || (Number.isInteger(value) && value >= 0 && value < 15); });
  var edgesOk = snap.edges.every(function (value, i) { return i % 3 !== 2 || (value % 7 === 0 && value < snap.nodes.length); });
  var root = snap.nodes.slice(0, 7);
  return [
    Object.keys(snap).join(','),
    JSON.stringify(s.meta) === NODE_META,
    Object.keys(s).join(','),
    s.node_count > 0,
    s.edge_count >= 0,
    snap.nodes.length === 7 * s.node_count,
    snap.edges.length === 3 * s.edge_count,
    edgesOk,
    nodeNamesOk,
    nodeTypesOk,
    Array.isArray(strings) && strings.every(function (name) { return typeof name === 'string'; }),
    root[0] === 9 && strings[root[1]] === '' && root[2] === 1 && root[5] === 0 && root[6] === 0,
    Object.keys(kinds).join(',')
  ];
};
"##,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_v8_snapshot_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(
        run_js(&bundle),
        r#"["snapshot,nodes,edges,trace_function_infos,trace_tree,samples,locations,strings",true,"meta,node_count,edge_count,trace_function_count",true,true,true,true,true,true,true,true,true,"string"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

// v8.getHeapSnapshot() streams 64 KiB Buffers by default (Node 22.23.1: on('data') gives 67 chunks, 65536 bytes each but the last)
#[test]
fn v8_get_heap_snapshot_streams_buffers_in_64_kib_chunks() {
    let dir = temp_registry("builtin_v8_snapshot_chunks");
    fs::write(
        dir.join("index.js"),
        r#"var v8 = require('node:v8'), once = require('node:events').once;
module.exports = async function () {
  var stream = v8.getHeapSnapshot(), chunks = [];
  stream.on('data', function (chunk) { chunks.push(chunk); });
  await once(stream, 'end');
  var text = Buffer.concat(chunks).toString('utf8');
  return [chunks.length > 1, chunks.every(Buffer.isBuffer), chunks.every(function (c) { return c.length <= 65536; }), JSON.parse(text).snapshot.node_count > 0];
};
"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_v8_snapshot_chunks_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), "[true,true,true,true]");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

// v8.writeHeapSnapshot(file) writes the snapshot text and returns the file name (Node 22.23.1)
#[test]
fn v8_write_heap_snapshot_writes_a_parseable_file() {
    let dir = temp_registry("builtin_v8_write_snapshot");
    let target = dir.join("out.heapsnapshot");
    fs::write(
        dir.join("index.js"),
        format!(
            "var v8 = require('node:v8'), fs = require('node:fs');\nmodule.exports = function () {{\n  var returned = v8.writeHeapSnapshot('{}');\n  var snap = JSON.parse(fs.readFileSync('{}', 'utf8'));\n  return [returned === '{}', typeof returned, snap.snapshot.node_count > 0, snap.nodes.length === 7 * snap.snapshot.node_count];\n}};\n",
            target.display(),
            target.display(),
            target.display()
        ),
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_v8_write_snapshot_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), "[true,\"string\",true,true]");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

// v8.writeHeapSnapshot() with no name: Heap.<YYYYMMDD>.<HHMMSS>.<pid>.<thread>.<seq>.heapsnapshot, seq counts per process
#[test]
fn v8_write_heap_snapshot_default_name_follows_node_pattern_and_increments() {
    let dir = temp_registry("builtin_v8_write_default");
    fs::write(
        dir.join("index.js"),
        r#"var v8 = require('node:v8'), fs = require('node:fs');
module.exports = function () {
  var first = v8.writeHeapSnapshot(), second = v8.writeHeapSnapshot();
  var pattern = /^Heap\.\d{8}\.\d{6}\.\d+\.0\.\d{3}\.heapsnapshot$/;
  var result = [pattern.test(first), pattern.test(second), first !== second, fs.existsSync(first), fs.existsSync(second)];
  fs.unlinkSync(first);
  fs.unlinkSync(second);
  return result;
};
"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_v8_write_default_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), "[true,true,true,true,true]");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

// v8.getHeapCodeStatistics() (Node 22.23.1: these four keys in this order; sizes > 0 except cpu_profiler_metadata_size)
#[test]
fn v8_heap_code_statistics_has_node_keys_and_positive_sizes() {
    let dir = temp_registry("builtin_v8_code");
    fs::write(
        dir.join("index.js"),
        "var v8 = require('node:v8'); module.exports = function () { var code = v8.getHeapCodeStatistics(); return [Object.keys(code).join(','), code.code_and_metadata_size > 0, code.bytecode_and_metadata_size > 0, code.external_script_source_size > 0, code.cpu_profiler_metadata_size === 0, code.code_and_metadata_size >= code.bytecode_and_metadata_size, v8.getHeapStatistics().total_heap_size_executable > 0]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_v8_code_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    // index.js plus the node:stream and node:fs modules the v8 module now requires (snapshot writer)
    assert_eq!(file_count, 5);
    assert_eq!(
        run_js(&bundle),
        r#"["code_and_metadata_size,bytecode_and_metadata_size,external_script_source_size,cpu_profiler_metadata_size",true,true,true,true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn v8_builtin_serializes_graphs_and_exposes_runtime_statistics() {
    let dir = temp_registry("builtin_v8");
    fs::write(
        dir.join("index.js"),
        "var v8 = require('node:v8');\n\
         module.exports = function () { var invalid = new Date(NaN), source = { bigint: 42n, bytes: Buffer.from('thaw'), map: new Map([['answer', 42]]), set: new Set(['x']), missing: undefined, valid: new Date(1234), invalid: invalid, invalidAgain: invalid, array: [invalid], dateMap: new Map([[invalid, invalid]]), dateSet: new Set([invalid]) }; source.self = source; var encoded = v8.serialize(source); var copy = v8.deserialize(encoded); if (copy.valid.getTime() !== 1234 || !Number.isNaN(copy.invalid.getTime()) || copy.invalid === invalid || copy.invalidAgain !== copy.invalid || copy.array[0] !== copy.invalid || copy.dateMap.get(copy.invalid) !== copy.invalid || !copy.dateSet.has(copy.invalid) || !Number.isNaN(v8.deserialize(v8.serialize(new Date(NaN))).getTime())) throw new Error('Date roundtrip'); var heap = v8.getHeapStatistics(); var code = v8.getHeapCodeStatistics(); return [Buffer.isBuffer(encoded), copy !== source, copy.self === copy, copy.bigint === 42n, copy.bytes.toString(), copy.map.get('answer'), copy.set.has('x'), Object.prototype.hasOwnProperty.call(copy, 'missing'), copy.missing === undefined, heap.number_of_native_contexts, heap.heap_size_limit > 0, v8.getHeapSpaceStatistics().length, code.code_and_metadata_size > 0, Number.isInteger(v8.cachedDataVersionTag()) && v8.cachedDataVersionTag() > 0 && v8.cachedDataVersionTag() <= 0xFFFFFFFF]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_v8_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    // index.js plus the node:stream and node:fs modules the v8 module now requires (snapshot writer)
    assert_eq!(file_count, 5);
    assert_eq!(
        run_js(&bundle),
        r#"[true,true,true,true,"thaw",42,true,true,true,1,true,11,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

// thaw-only: v8.serialize emits thaw's JSON wire format (Node emits the binary ValueSerializer format) and a malformed
// buffer throws SyntaxError (Node throws a plain Error). See designs/worker-diagnostics-v8.md D1.
#[test]
fn v8_thaw_json_wire_format_and_legacy_decode_is_thaw_only() {
    let dir = temp_registry("builtin_v8_thaw_wire");
    fs::write(
        dir.join("index.js"),
        "var v8 = require('node:v8');\n\
         module.exports = function () { var invalid = new Date(NaN), source = { valid: new Date(1234), invalid: invalid, array: [invalid] }; var encoded = v8.serialize(source); var wire = JSON.parse(encoded.toString()); var legacy = v8.deserialize(Buffer.from(JSON.stringify({ root: { t: 'r', v: 0 }, nodes: [{ k: 'd', v: '1970-01-01T00:00:00.000Z' }] }))); var dateNodes = wire.nodes.filter(function(node) { return node.k === 'd'; }); var malformedCaught = false; try { v8.deserialize(Buffer.from('{')); } catch (error) { malformedCaught = error instanceof SyntaxError; } return [dateNodes.some(function(node) { return node.v.t === 'p' && node.v.v === 1234; }), dateNodes.some(function(node) { return node.v.t === 'n' && node.v.v === 'NaN'; }), legacy.getTime() === 0, malformedCaught]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_v8_thaw_wire_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), r#"[true,true,true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn zlib_builtin_compresses_sync_and_callback_values() {
        let dir = temp_registry("builtin_zlib");
    fs::write(dir.join("index.js"), "var zlib = require('node:zlib'), stream = require('node:stream'); module.exports = async function () { var source = Buffer.from('thaw compression '.repeat(8)); var gzip = zlib.gzipSync(source); var raw = zlib.deflateRawSync(source); var brotli = zlib.brotliCompressSync(source); var callback = await new Promise(function(resolve, reject) { zlib.gunzip(gzip, function(error, value) { error ? reject(error) : resolve(value); }); }); var brotliCallback = await new Promise(function(resolve, reject) { zlib.brotliDecompress(brotli, function(error, value) { error ? reject(error) : resolve(value); }); }); var compressor = zlib.createGzip(), compressed = []; compressor.on('data', function(value) { compressed.push(value); }); var streamDone = new Promise(function(resolve, reject) { compressor.on('error', reject); compressor.on('end', resolve); }); compressor.write(source.subarray(0, 7)); compressor.end(source.subarray(7)); await streamDone; var streamed = Buffer.concat(compressed), decompressor = zlib.createGunzip(), restored = []; decompressor.on('data', function(value) { restored.push(value); }); var restoreDone = new Promise(function(resolve, reject) { decompressor.on('error', reject); decompressor.on('end', resolve); }); stream.Readable.from([streamed.subarray(0, 5), streamed.subarray(5)]).pipe(decompressor); await restoreDone; var brotliStream = zlib.createBrotliCompress(), brotliChunks = []; brotliStream.on('data', function(value) { brotliChunks.push(value); }); var brotliDone = new Promise(function(resolve, reject) { brotliStream.on('error', reject); brotliStream.on('end', resolve); }); brotliStream.end(source); await brotliDone; var flushed = await new Promise(function(resolve) { compressor.flush(resolve); }); function released(owner, handle) { return owner._streamHandle === null && __thaw_zlib_stream_drop(handle) === false; } var owned = zlib.createGzip({ autoDestroy: false }), ownedHandle = owned._streamHandle; owned.on('data', function() {}); var ownedDone = new Promise(function(resolve, reject) { owned.on('error', reject); owned.end(source, resolve); }); await ownedDone; if (!released(owned, ownedHandle)) throw new Error('zlib buffered completion release'); var malformed = zlib.createGunzip({ autoDestroy: false }), malformedHandle = malformed._streamHandle; malformed.resume(); var malformedError = await new Promise(function(resolve) { malformed.once('error', resolve); malformed.end(Buffer.from('not gzip')); }); if (malformedError.code !== 'Z_DATA_ERROR' || !released(malformed, malformedHandle)) throw new Error('zlib malformed completion release'); var early = zlib.createGzip({ autoDestroy: false }), earlyHandle = early._streamHandle; early.destroy(); if (!released(early, earlyHandle)) throw new Error('zlib early destroy release'); var closeCalls = 0; await new Promise(function(resolve) { early.close(function() { closeCalls++; resolve(); }); }); await new Promise(function(resolve) { early.close(function() { closeCalls++; resolve(); }); }); early.destroy(); if (closeCalls !== 2 || !released(early, earlyHandle)) throw new Error('zlib repeated close'); var deliberate = new Error('intentional destroy'), destroyed = zlib.createGzip({ autoDestroy: false }), destroyedHandle = destroyed._streamHandle; var destroyedError = await new Promise(function(resolve) { destroyed.once('error', resolve); destroyed.destroy(deliberate); }); if (destroyedError !== deliberate || !released(destroyed, destroyedHandle)) throw new Error('zlib destroy error identity'); var flushRejected = (function() { try { zlib.createGzip({ autoDestroy: false, flush: function(callback) { callback(); } }); return null; } catch (error) { return error.code; } })(); if (flushRejected !== 'ERR_INVALID_ARG_TYPE') throw new Error('zlib custom flush option'); var destroyCalls = 0, destroyReceiver, customDestroy = zlib.createGzip({ autoDestroy: false, destroy: function(error, callback) { destroyCalls++; destroyReceiver = this; this.destroySeen = error; callback(error); } }), customDestroyHandle = customDestroy._streamHandle, customDestroyError = new Error('custom destroy'); var customDestroySeen = await new Promise(function(resolve) { customDestroy.once('error', resolve); customDestroy.destroy(customDestroyError); }); if (destroyCalls !== 1 || destroyReceiver !== customDestroy || customDestroySeen !== customDestroyError || customDestroy.destroySeen !== customDestroyError || !released(customDestroy, customDestroyHandle)) throw new Error('zlib custom destroy release'); var incremental = new zlib.Gzip({ autoDestroy: false }), incrementalHandle = incremental._streamHandle; incremental._processChunk(source, zlib.constants.Z_NO_FLUSH); incremental._processChunk(Buffer.alloc(0), zlib.constants.Z_FINISH); incremental.destroy(); if (!released(incremental, incrementalHandle)) throw new Error('zlib incremental finish release'); var emptyUnzip = new zlib.Unzip({ autoDestroy: false }); await new Promise(function(resolve) { emptyUnzip.once('close', resolve); emptyUnzip.destroy(); }); if (emptyUnzip._streamHandle !== null) throw new Error('zlib empty unzip destroy'); var lazyUnzip = new zlib.Unzip({ autoDestroy: false }); lazyUnzip._processChunk(gzip, zlib.constants.Z_NO_FLUSH); var lazyHandle = lazyUnzip._streamHandle; lazyUnzip.destroy(); if (!released(lazyUnzip, lazyHandle)) throw new Error('zlib lazy unzip destroy'); return [gzip[0], gzip[1], zlib.gunzipSync(gzip).toString() === source.toString(), zlib.inflateRawSync(raw).toString() === source.toString(), callback.toString() === source.toString(), zlib.constants.Z_OK, zlib.constants.Z_SYNC_FLUSH, compressor instanceof zlib.Gzip, compressor instanceof stream.Transform, compressor.bytesWritten, Buffer.concat(restored).toString() === source.toString(), flushed, zlib.brotliDecompressSync(brotli).toString() === source.toString(), brotliCallback.toString() === source.toString(), brotliStream instanceof zlib.BrotliCompress, zlib.brotliDecompressSync(Buffer.concat(brotliChunks)).toString() === source.toString(), zlib.constants.BROTLI_OPERATION_FINISH]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_zlib_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let result = run_js(&bundle);
    assert_eq!(
        result,
        r#"[31,139,true,true,true,0,2,true,true,136,true,null,true,true,true,true,2]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

/// A real npm package (`crc32-stream`, a dependency of `archiver`) does
/// `class DeflateCRC32Stream extends zlib.DeflateRaw { _transform(chunk,
/// ...) { ...; super._transform(chunk, ...); } }` -- a genuine, real-
/// world subclass overriding `_transform` to observe every chunk while
/// still delegating to the real compression. Two bugs, both found
/// investigating why the resulting zip's own CRC-32/size metadata came
/// back all zero even though the compressed *content* round-tripped
/// correctly:
///
/// 1. `ZlibTransform` (`system/runtime.rs`) built every zlib stream
///    class (`Gzip`/`Deflate`/`DeflateRaw`/...) by passing `{transform:
///    fn, flush: fn}` as *constructor options* to the base `Transform`
///    class, which (matching real Node's own documented behavior for
///    that pattern) sets `this._transform`/`this._flush` as *instance*
///    properties -- shadowing a subclass's own *prototype* method of the
///    same name entirely, so the override was silently never called.
///    Real Node's own internal zlib classes don't use this pattern
///    (matching an even a bare userland `Zlib.prototype._transform =
///    ...` isn't overridden this way), which is exactly what let
///    `crc32-stream` correctly subclass them there. Fixed by moving
///    `_transform`/`_flush` onto `ZlibTransform.prototype` directly.
/// 2. Fixing bug 1 alone still hung (`'end'` never fired): the fix's own
///    accumulator for pending raw bytes was named `this._chunks` --
///    colliding with `Readable`'s *own* internal buffered-but-unread
///    data queue, also named `_chunks` on the very same instance (a
///    `ZlibTransform` is both a `Readable` and a `Writable` via
///    `Duplex`). Pushing compressed output into the (corrupted, shared)
///    array left `stream._chunks.length` non-zero forever, so
///    `emitReadableEnd`'s own "no pending data left" check never passed.
///    Fixed by renaming the accumulator to `_zlibChunks`.
#[test]
fn a_zlib_streams_subclass_overriding_transform_is_actually_called() {
        let dir = temp_registry("builtin_zlib_subclass");
    fs::write(
        dir.join("index.js"),
        "var zlib = require('node:zlib'); \
         function MyDeflate() { zlib.DeflateRaw.call(this); this.calls = 0; this.observedBytes = 0; } \
         MyDeflate.prototype = Object.create(zlib.DeflateRaw.prototype); \
         MyDeflate.prototype.constructor = MyDeflate; \
         MyDeflate.prototype._transform = function(chunk, encoding, callback) { \
         \x20\x20this.calls++; \
         \x20\x20this.observedBytes += chunk ? chunk.length : 0; \
         \x20\x20zlib.DeflateRaw.prototype._transform.call(this, chunk, encoding, callback); \
         }; \
         module.exports = async function () { \
         \x20\x20var d = new MyDeflate(), out = []; \
         \x20\x20d.on('data', function(value) { out.push(value); }); \
         \x20\x20var done = new Promise(function(resolve, reject) { d.on('error', reject); d.on('end', resolve); }); \
         \x20\x20d.write(Buffer.from('hello ')); \
         \x20\x20d.end(Buffer.from('world')); \
         \x20\x20await done; \
         \x20\x20var restored = zlib.inflateRawSync(Buffer.concat(out)); \
         \x20\x20return [d.calls > 0, d.observedBytes, restored.toString()]; \
         };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_zlib_subclass_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let result = run_js(&bundle);
    assert_eq!(result, r#"[true,11,"hello world"]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

/// Real `tar` with `gzip: true` crashed (`cannot read property 'close'
/// of undefined`) because its dependency `minizlib` never uses the
/// ordinary Transform-stream API (`.write()`/`.pipe()`) on a
/// `zlib.Gzip`/`Gunzip` instance at all -- it constructs one purely to
/// reach into its *private* synchronous-compression contract, the same
/// one real Node's own C++ zlib binding exposes. That contract is a
/// `._handle` object (temporarily neutered via `._handle.close`/
/// `.close()` around each call, to stop the real binding from tearing
/// itself down mid-operation) plus a `._processChunk(chunk, flushFlag)`
/// method that synchronously compresses/decompresses one chunk and
/// returns the newly produced output bytes directly, with no events and
/// no stream machinery involved. Neither existed on thaw's
/// `ZlibTransform` -- reading `._handle` off `undefined` (there was no
/// `._handle` at all) is exactly the observed crash.
///
/// Fixed in `crates/thaw-registry/src/registry/builtins/system/
/// runtime.rs`: `ZlibTransform` now allocates a real incremental
/// compression stream via the same native `__thaw_zlib_stream_create`/
/// `_write`/`_drop` bridge the Web `CompressionStream` API already uses
/// (`WebZlibStream`, thaw-quickjs), and exposes it through `._handle`
/// (a harmless `{ close: fn() {} }` stub -- there's no real native
/// handle to actually own) and `._processChunk`. Also added `zlib.Unzip`/
/// `createUnzip` (real Node's auto-detecting gzip-or-deflate decompressor,
/// used by tar's *extraction* side) -- it didn't exist here at all
/// (`Compression method not supported: Unzip`), sniffing the gzip magic
/// bytes (`0x1f 0x8b`) on the first chunk to lazily pick the real format.
#[test]
fn zlib_gzip_exposes_the_private_handle_and_process_chunk_contract_minizlib_needs() {
        let dir = temp_registry("builtin_zlib_handle");
    fs::write(
        dir.join("index.js"),
        "var zlib = require('node:zlib'); \
         module.exports = async function () { \
         \x20\x20var source = Buffer.from('thaw compression '.repeat(64)); \
         \x20\x20var gzip = new zlib.Gzip(); \
         \x20\x20var handle = gzip._handle, closeStub = handle.close; \
         \x20\x20handle.close = function() {}; \
         \x20\x20var first = gzip._processChunk(source, zlib.constants.Z_NO_FLUSH); \
         \x20\x20var last = gzip._processChunk(Buffer.alloc(0), zlib.constants.Z_FINISH); \
         \x20\x20gzip._handle = handle; \
         \x20\x20handle.close = closeStub; \
         \x20\x20var compressed = Buffer.concat([first, last]); \
         \x20\x20var restored = zlib.gunzipSync(compressed); \
         \x20\x20var unzip = new zlib.Unzip(); \
         \x20\x20var unzipped = Buffer.concat([unzip._processChunk(compressed, zlib.constants.Z_NO_FLUSH), unzip._processChunk(Buffer.alloc(0), zlib.constants.Z_FINISH)]); \
         \x20\x20return [compressed[0], compressed[1], restored.toString() === source.toString(), unzipped.toString() === source.toString()]; \
         };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_zlib_handle_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let result = run_js(&bundle);
    assert_eq!(result, r#"[31,139,true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn zlib_reset_discards_buffers_and_recreates_native_state() {
    std::thread::spawn(|| {

        let dir = temp_registry("builtin_zlib_reset");
        fs::write(
            dir.join("index.js"),
            r#"
var zlib = require('node:zlib');
module.exports = async function() {
  var streams = [], createBase = globalThis.__thaw_zlib_stream_create;
  var createCalls = 0;
  globalThis.__thaw_zlib_stream_create = function(operation, format) {
    createCalls++;
    return createBase(operation, format);
  };
  function keep(stream) { streams.push(stream); return stream; }
  function rejectedReset(stream) {
    try { stream.reset(); return false; }
    catch (error) { return error.message === 'Cannot reset a finished zlib stream'; }
  }
  function dropped(handle) { return __thaw_zlib_stream_drop(handle) === false; }
  var bufferedText, bufferedBytes, bufferedSame, bufferedOldGone, bufferedHandleGone;
  var deflateText, deflateOldGone, deflateHandleGone;
  var inflateText, inflateOldGone;
  var autoUnopened, autoResetNull, autoNoCreate, autoOldGone, autoText, autoHandleGone, autoTerminal;
  var fixedTerminal, fixedNoCreate, truncatedFailed, truncatedErrorCaptured, truncatedReleased, truncatedTerminal, truncatedNoCreate;
  var normalFinalTerminal, normalFinalReleased, resetCloseReleased, resetCloseTerminal, destroyTerminal;
  var createFailureIdentity, createFailureKeepsHandle, createFailureContinues, createFailureReleased;
  var bufferedCreateFailureIdentity, bufferedCreateFailureKeepsHandle, bufferedCreateFailureText, bufferedCreateFailureReleased;
  try {
    var buffered = keep(zlib.createGzip({ autoDestroy: false }));
    var bufferedOld = buffered._streamHandle, bufferedChunks = [];
    buffered.on('data', function(chunk) { bufferedChunks.push(chunk); });
    buffered.write(Buffer.from('old'));
    bufferedSame = buffered.reset() === buffered;
    bufferedOldGone = dropped(bufferedOld);
    var bufferedNew = buffered._streamHandle;
    bufferedBytes = -1;
    await new Promise(function(resolve, reject) {
      buffered.on('error', reject);
      buffered.end(Buffer.from('new'), resolve);
    });
    bufferedText = zlib.gunzipSync(Buffer.concat(bufferedChunks)).toString();
    bufferedBytes = buffered.bytesWritten;
    bufferedHandleGone = dropped(bufferedNew);
    normalFinalTerminal = rejectedReset(buffered);
    normalFinalReleased = bufferedHandleGone;

    var deflater = keep(new zlib.Deflate({ autoDestroy: false }));
    var deflateOld = deflater._streamHandle;
    deflater._processChunk(Buffer.from('discarded-'), zlib.constants.Z_NO_FLUSH);
    deflater.reset();
    deflateOldGone = dropped(deflateOld);
    var deflateNew = deflater._streamHandle;
    var deflated = Buffer.concat([
      deflater._processChunk(Buffer.from('fresh deflate'), zlib.constants.Z_NO_FLUSH),
      deflater._processChunk(Buffer.alloc(0), zlib.constants.Z_FINISH)
    ]);
    deflateText = zlib.inflateSync(deflated).toString();
    deflateHandleGone = dropped(deflateNew);

    var inflater = keep(new zlib.Inflate({ autoDestroy: false }));
    var inflateOld = inflater._streamHandle;
    inflater._processChunk(zlib.deflateSync(Buffer.from('discarded inflate')), zlib.constants.Z_NO_FLUSH);
    inflater.reset();
    inflateOldGone = dropped(inflateOld);
    var inflated = Buffer.concat([
      inflater._processChunk(zlib.deflateSync(Buffer.from('fresh inflate')), zlib.constants.Z_NO_FLUSH),
      inflater._processChunk(Buffer.alloc(0), zlib.constants.Z_FINISH)
    ]);
    inflateText = inflated.toString();

    var auto = keep(new zlib.Unzip({ autoDestroy: false }));
    var beforeAutoReset = createCalls;
    autoUnopened = auto._streamHandle === null;
    autoResetNull = auto.reset() === auto && auto._streamHandle === null;
    autoNoCreate = createCalls === beforeAutoReset;
    auto._processChunk(zlib.gzipSync(Buffer.from('discarded auto')), zlib.constants.Z_NO_FLUSH);
    var autoOld = auto._streamHandle;
    auto.reset();
    autoOldGone = dropped(autoOld);
    var autoFirst = auto._processChunk(zlib.deflateSync(Buffer.from('auto deflate')), zlib.constants.Z_NO_FLUSH);
    var autoNew = auto._streamHandle;
    var autoOutput = Buffer.concat([autoFirst, auto._processChunk(Buffer.alloc(0), zlib.constants.Z_FINISH)]);
    autoText = autoOutput.toString();
    autoHandleGone = dropped(autoNew);
    var autoCreatesBeforeReset = createCalls;
    autoTerminal = rejectedReset(auto) && createCalls === autoCreatesBeforeReset;

    var fixed = keep(new zlib.Gzip({ autoDestroy: false }));
    fixed._processChunk(Buffer.from('finished'), zlib.constants.Z_FINISH);
    var fixedCreatesBeforeReset = createCalls;
    fixedTerminal = rejectedReset(fixed);
    fixedNoCreate = createCalls === fixedCreatesBeforeReset;

    var truncated = keep(new zlib.Gunzip({ autoDestroy: false }));
    var truncatedHandle = truncated._streamHandle;
    var truncatedGzip = zlib.gzipSync(Buffer.from('truncated payload'));
    var truncatedInput = truncatedGzip.subarray(0, truncatedGzip.length - 4);
    var truncatedError;
    try { truncated._processChunk(truncatedInput, zlib.constants.Z_FINISH); }
    catch (error) { truncatedError = error; truncatedFailed = true; }
    truncatedErrorCaptured = truncatedError instanceof Error && truncatedError.message.length > 0;
    truncatedReleased = dropped(truncatedHandle);
    var truncatedCreatesBeforeReset = createCalls;
    truncatedTerminal = rejectedReset(truncated);
    truncatedNoCreate = createCalls === truncatedCreatesBeforeReset;

    var closeAfterReset = keep(new zlib.Gzip({ autoDestroy: false }));
    closeAfterReset.reset();
    var closeReplacement = closeAfterReset._streamHandle;
    closeAfterReset.close();
    resetCloseReleased = dropped(closeReplacement);
    resetCloseTerminal = rejectedReset(closeAfterReset);

    var destroyed = keep(new zlib.Gzip({ autoDestroy: false }));
    destroyed.destroy();
    destroyTerminal = rejectedReset(destroyed);

    var bufferedFailure = keep(new zlib.Gzip({ autoDestroy: false }));
    var bufferedFailureHandle = bufferedFailure._streamHandle, bufferedFailureChunks = [];
    bufferedFailure.on('data', function(chunk) { bufferedFailureChunks.push(chunk); });
    bufferedFailure.write(Buffer.from('old-'));
    var trackedCreate = globalThis.__thaw_zlib_stream_create;
    var sentinel = new Error('create sentinel');
    try {
      globalThis.__thaw_zlib_stream_create = function() { throw sentinel; };
      try { bufferedFailure.reset(); }
      catch (error) { bufferedCreateFailureIdentity = error === sentinel; }
    } finally {
      globalThis.__thaw_zlib_stream_create = trackedCreate;
    }
    bufferedCreateFailureKeepsHandle = bufferedFailure._streamHandle === bufferedFailureHandle;
    await new Promise(function(resolve, reject) {
      bufferedFailure.on('error', reject);
      bufferedFailure.end(Buffer.from('new'), resolve);
    });
    bufferedCreateFailureText = zlib.gunzipSync(Buffer.concat(bufferedFailureChunks)).toString();
    bufferedCreateFailureReleased = dropped(bufferedFailureHandle);

    var continuing = keep(new zlib.Deflate({ autoDestroy: false }));
    var continuingHandle = continuing._streamHandle;
    var oldOutput = continuing._processChunk(Buffer.from('old-'), zlib.constants.Z_NO_FLUSH);
    trackedCreate = globalThis.__thaw_zlib_stream_create;
    try {
      globalThis.__thaw_zlib_stream_create = function() { throw sentinel; };
      try { continuing.reset(); }
      catch (error) { createFailureIdentity = error === sentinel; }
    } finally {
      globalThis.__thaw_zlib_stream_create = trackedCreate;
    }
    createFailureKeepsHandle = continuing._streamHandle === continuingHandle;
    var continued = Buffer.concat([
      oldOutput,
      continuing._processChunk(Buffer.from('new'), zlib.constants.Z_NO_FLUSH),
      continuing._processChunk(Buffer.alloc(0), zlib.constants.Z_FINISH)
    ]);
    createFailureContinues = zlib.inflateSync(continued).toString();
    createFailureReleased = dropped(continuingHandle);
  } finally {
    globalThis.__thaw_zlib_stream_create = createBase;
    for (var index = 0; index < streams.length; index++) {
      try { streams[index].destroy(); } catch (_) {}
    }
  }
  return {
    bufferedText: bufferedText, bufferedBytes: bufferedBytes, bufferedSame: bufferedSame,
    bufferedOldGone: bufferedOldGone, bufferedHandleGone: bufferedHandleGone,
    deflateText: deflateText, deflateOldGone: deflateOldGone, deflateHandleGone: deflateHandleGone,
    inflateText: inflateText, inflateOldGone: inflateOldGone,
    autoUnopened: autoUnopened, autoResetNull: autoResetNull, autoNoCreate: autoNoCreate,
    autoOldGone: autoOldGone, autoText: autoText, autoHandleGone: autoHandleGone, autoTerminal: autoTerminal,
    fixedTerminal: fixedTerminal, fixedNoCreate: fixedNoCreate,
    truncatedFailed: truncatedFailed, truncatedErrorCaptured: truncatedErrorCaptured, truncatedReleased: truncatedReleased, truncatedTerminal: truncatedTerminal, truncatedNoCreate: truncatedNoCreate,
    normalFinalTerminal: normalFinalTerminal, normalFinalReleased: normalFinalReleased,
    resetCloseReleased: resetCloseReleased, resetCloseTerminal: resetCloseTerminal, destroyTerminal: destroyTerminal,
    createFailureIdentity: createFailureIdentity, createFailureKeepsHandle: createFailureKeepsHandle,
    createFailureContinues: createFailureContinues, createFailureReleased: createFailureReleased,
    bufferedCreateFailureIdentity: bufferedCreateFailureIdentity, bufferedCreateFailureKeepsHandle: bufferedCreateFailureKeepsHandle,
    bufferedCreateFailureText: bufferedCreateFailureText, bufferedCreateFailureReleased: bufferedCreateFailureReleased
  };
};
"#,
        )
        .unwrap();
        let empty_node_modules = temp_registry("builtin_zlib_reset_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
        let result = run_js(&bundle);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&empty_node_modules);
        assert_eq!(file_count, 3);
        assert_eq!(
            result,
            r#"{"bufferedText":"oldnew","bufferedBytes":6,"bufferedSame":true,"bufferedOldGone":true,"bufferedHandleGone":true,"deflateText":"fresh deflate","deflateOldGone":true,"deflateHandleGone":true,"inflateText":"fresh inflate","inflateOldGone":true,"autoUnopened":true,"autoResetNull":true,"autoNoCreate":true,"autoOldGone":true,"autoText":"auto deflate","autoHandleGone":true,"autoTerminal":true,"fixedTerminal":true,"fixedNoCreate":true,"truncatedFailed":true,"truncatedErrorCaptured":true,"truncatedReleased":true,"truncatedTerminal":true,"truncatedNoCreate":true,"normalFinalTerminal":true,"normalFinalReleased":true,"resetCloseReleased":true,"resetCloseTerminal":true,"destroyTerminal":true,"createFailureIdentity":true,"createFailureKeepsHandle":true,"createFailureContinues":"old-new","createFailureReleased":true,"bufferedCreateFailureIdentity":true,"bufferedCreateFailureKeepsHandle":true,"bufferedCreateFailureText":"old-new","bufferedCreateFailureReleased":true}"#
        );
    })
    .join()
    .unwrap();
}


#[test]
fn worker_threads_builtin_exchanges_cloned_messages() {
    let dir = temp_registry("builtin_worker_threads");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var channel = new workers.MessageChannel(); var original = { value: 7 }; channel.port1.postMessage(original); original.value = 9; var received = workers.receiveMessageOnPort(channel.port2); var pending = new Promise(function(resolve) { channel.port2.once('message', resolve); }); channel.port1.postMessage(new Map([['answer', 42]])); var asynchronous = await pending; workers.setEnvironmentData('config', { enabled: true }); var environment = workers.getEnvironmentData('config'); environment.enabled = false; var freshEnvironment = workers.getEnvironmentData('config'); var protectedBuffer = new ArrayBuffer(4); workers.markAsUntransferable(protectedBuffer); var transferRejected = false; try { channel.port1.postMessage('x', [protectedBuffer]); } catch (error) { transferRejected = error.name === 'DataCloneError'; } var transferStillAttached = protectedBuffer.byteLength === 4; var cloneTransferRejected = false; try { structuredClone(protectedBuffer, { transfer: [protectedBuffer] }); } catch (error) { cloneTransferRejected = error.name === 'DataCloneError'; } var cloneTransferStillAttached = protectedBuffer.byteLength === 4; var workerTransferRejected = false; try { new workers.Worker(\"void 0;\", { eval: true, transferList: [protectedBuffer] }); } catch (error) { workerTransferRejected = error.name === \"DataCloneError\"; } var workerTransferStillAttached = protectedBuffer.byteLength === 4; var protectedObject = { value: 1 }; workers.markAsUncloneable(protectedObject); var cloneRejected = false; try { structuredClone({ nested: protectedObject }); } catch (error) { cloneRejected = error.name === 'DataCloneError'; } var cloneReads = 0, cloneGetter = structuredClone({ get value() { cloneReads++; return cloneReads; } }); var portReads = 0; channel.port1.postMessage({ get value() { portReads++; return portReads; } }); var portGetter = workers.receiveMessageOnPort(channel.port2); var nestedReads = 0, nestedRejected = false; try { structuredClone({ get nested() { nestedReads++; return protectedObject; } }); } catch (error) { nestedRejected = error.name === 'DataCloneError'; } var cycle = {}; cycle.self = cycle; cycle.marked = protectedObject; var mapRejected = false, setRejected = false; try { structuredClone(new Map([[cycle, 1]])); } catch (error) { mapRejected = error.name === 'DataCloneError'; } try { structuredClone(new Set([cycle])); } catch (error) { setRejected = error.name === 'DataCloneError'; } var portMarkedRejected = false; try { channel.port1.postMessage({ child: protectedObject }); } catch (error) { portMarkedRejected = error.name === 'DataCloneError'; } var extra = new workers.MessageChannel(); channel.port1.postMessage({ port: extra.port1 }, [extra.port1]); var transferred = workers.receiveMessageOnPort(channel.port2); transferred.message.port.postMessage('through'); var through = workers.receiveMessageOnPort(extra.port2).message; var broadcastSender = new workers.BroadcastChannel('sync-receive'), broadcastReceiver = new workers.BroadcastChannel('sync-receive'); broadcastSender.postMessage({ value: 5 }); var broadcast = workers.receiveMessageOnPort(broadcastReceiver); broadcastSender.close(); broadcastReceiver.close(); extra.port1.close(); extra.port2.close(); channel.port1.unref(); var refed = channel.port1.hasRef(); channel.port1.ref(); channel.port1.close(); channel.port2.close(); return [workers.isMainThread, workers.threadId, workers.parentPort, received.message.value, asynchronous.get('answer'), freshEnvironment.enabled, refed, channel.port1.hasRef(), workers.SHARE_ENV === Symbol.for('nodejs.worker_threads.SHARE_ENV'), workers.receiveMessageOnPort(channel.port2) === undefined, workers.isMarkedAsUntransferable(protectedBuffer), transferRejected, transferStillAttached, cloneTransferRejected, cloneTransferStillAttached, workerTransferRejected, workerTransferStillAttached, cloneRejected, cloneReads, cloneGetter.value, portReads, portGetter.message.value, nestedReads, nestedRejected, mapRejected, setRejected, portMarkedRejected, broadcast.message.value, through]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_worker_threads_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(
        result,
        r#"[true,0,null,7,42,false,false,true,true,true,true,true,true,true,true,true,true,true,1,1,1,1,1,true,true,true,true,5,"through"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn bundled_commonjs_require_exposes_resolve() {
    let dir = temp_registry("commonjs_require_resolve");
    fs::write(
        dir.join("index.js"),
        "var dependency = require('./dependency'); module.exports = function () { return [dependency, typeof require.resolve, require.resolve('./dependency')]; };",
    )
    .unwrap();
    fs::write(dir.join("dependency.js"), "module.exports = 42;").unwrap();
    let node_modules = temp_registry("commonjs_require_resolve_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(
        run_js(&bundle),
        r#"[42,"function","/thaw_modules/pkg/dependency.js"]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}


#[test]
fn worker_threads_eval_worker_isolates_state_and_exchanges_messages() {
    let dir = temp_registry("builtin_worker_eval");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var events = []; delete process.env.THAW_WORKER_TEST; var source = \"var wt = require('node:worker_threads'); globalThis.workerOnly = 99; wt.parentPort.on('message', function(value) { var args = process.argv.slice(-2).join(','); var environment = process.env.THAW_WORKER_TEST; process.env.THAW_WORKER_TEST = 'mutated'; wt.parentPort.postMessage({ answer: wt.workerData.base + value, main: wt.isMainThread, threadId: wt.threadId, threadName: wt.threadName, isolated: globalThis.workerOnly, args: args, execArgv: process.execArgv.join(','), environment: environment }); wt.parentPort.close(); });\"; var worker = new workers.Worker(source, { eval: true, name: 'alpha', workerData: { base: 40 }, argv: ['one', 2], execArgv: ['--trace-warnings'], env: { THAW_WORKER_TEST: 'child' }, resourceLimits: { maxOldGenerationSizeMb: 64 } }); var listenerOrder = []; function regular() { listenerOrder.push('regular'); } worker.addListener('probe', regular).prependOnceListener('probe', function() { listenerOrder.push('first'); }); var listenerCount = worker.listenerCount('probe'); var hasProbe = worker.eventNames().indexOf('probe') >= 0; worker.emit('probe'); worker.emit('probe'); worker.removeListener('probe', regular).setMaxListeners(20); var completed = new Promise(function(resolve) { worker.on('online', function() { events.push('online'); }); worker.on('message', function(value) { events.push('message:' + value.answer + ':' + value.main + ':' + (value.threadId > 0) + ':' + value.threadName + ':' + value.isolated + ':' + value.args + ':' + value.execArgv + ':' + value.environment); }); worker.on('error', function(error) { events.push('error:' + error.stack); resolve(); }); worker.on('exit', function(code) { events.push('exit:' + code); resolve(); }); }); worker.postMessage(2); await completed; delete process.env.THAW_WORKER_SHARED; var shared = new workers.Worker(\"var wt = require('node:worker_threads'); process.env.THAW_WORKER_SHARED = 'shared'; wt.parentPort.close();\", { eval: true, env: workers.SHARE_ENV }); await new Promise(function(resolve, reject) { shared.on('error', reject); shared.on('exit', resolve); }); var sharedValue = process.env.THAW_WORKER_SHARED; delete process.env.THAW_WORKER_SHARED; return [events, worker.threadName, listenerOrder, listenerCount, hasProbe, worker.listenerCount('probe'), worker.getMaxListeners(), typeof globalThis.workerOnly, process.env.THAW_WORKER_TEST, sharedValue, worker.resourceLimits.maxOldGenerationSizeMb, worker.threadId > 0, worker.ref() === worker, worker.unref() === worker, await worker.terminate()]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_worker_eval_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    // Node 22: worker.threadId is -1 once the worker has exited, so `worker.threadId > 0` is false after `await completed`.
    assert_eq!(
        result,
        r#"[["online","message:42:false:true:alpha:99:one,2:--trace-warnings:child","exit:0"],null,["first","regular","regular"],2,true,0,20,"undefined",null,"shared",null,false,false,false,null]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_threads_share_environment_across_native_workers() {
    let dir = temp_registry("builtin_worker_native_share_env");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); } function result(worker) { return new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); }); } module.exports = async function () { delete process.env.THAW_PARENT_SHARED; delete process.env.THAW_WORKER_SHARED; delete process.env.THAW_SECOND_SHARED; var first = new workers.Worker(\"var wt = require('node:worker_threads'); wt.parentPort.on('message', function() { var parent = process.env.THAW_PARENT_SHARED; process.env.THAW_WORKER_SHARED = 'worker'; wt.parentPort.postMessage(parent); wt.parentPort.close(); });\", { eval: true, env: workers.SHARE_ENV }); await online(first); process.env.THAW_PARENT_SHARED = 'parent'; var firstResult = result(first), firstExit = new Promise(function(resolve) { first.on('exit', resolve); }); first.postMessage('go'); var parentSeen = await firstResult; await firstExit; var second = new workers.Worker(\"var wt = require('node:worker_threads'); wt.parentPort.postMessage(process.env.THAW_WORKER_SHARED); process.env.THAW_SECOND_SHARED = 'second'; wt.parentPort.close();\", { eval: true, env: workers.SHARE_ENV }); var secondResult = result(second), secondExit = new Promise(function(resolve) { second.on('exit', resolve); }); var workerSeen = await secondResult; await secondExit; var secondSeen = process.env.THAW_SECOND_SHARED; delete process.env.THAW_PARENT_SHARED; delete process.env.THAW_WORKER_SHARED; delete process.env.THAW_SECOND_SHARED; return [parentSeen, workerSeen, secondSeen]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_native_share_env_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(
        result,
        r#"["parent","worker","second"]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}


#[test]
fn worker_threads_moves_message_ports_to_vm_contexts() {
    let dir = temp_registry("builtin_worker_move_port_context");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); var vm = require('node:vm'); module.exports = function () { var channel = new workers.MessageChannel(); var context = vm.createContext({}); var moved = workers.moveMessagePortToContext(channel.port1, context); moved.postMessage('moved'); var received = workers.receiveMessageOnPort(channel.port2); var invalidContext = false; try { workers.moveMessagePortToContext(channel.port2, {}); } catch (error) { invalidContext = error instanceof TypeError; } return [received.message, moved !== channel.port1, invalidContext]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_move_port_context_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 5);
    let result = run_js(&bundle);
    assert_eq!(result, r#"["moved",true,true]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_transfers_worker_data_message_ports() {
    let dir = temp_registry("builtin_worker_data_transfer");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); module.exports = async function () { var channel = new workers.MessageChannel(); var source = \"var wt = require('node:worker_threads'); wt.workerData.port.postMessage('from-worker'); wt.workerData.port.on('message', function(value) { wt.workerData.port.postMessage(value.toUpperCase()); wt.workerData.port.close(); wt.parentPort.close(); });\"; var worker = new workers.Worker(source, { eval: true, workerData: { port: channel.port1 }, transferList: [channel.port1] }); var first = new Promise(function(resolve) { channel.port2.once('message', resolve); }); var exit = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); var initial = await first, second = new Promise(function(resolve) { channel.port2.once('message', resolve); }); channel.port2.postMessage('through'); return [initial, await second, await exit]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_data_transfer_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"["from-worker","THROUGH",0]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_transfer_message_ports_from_native_workers() {
    let dir = temp_registry("builtin_worker_native_outbound_port");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = \"var wt = require('node:worker_threads'), channel = new wt.MessageChannel(); channel.port2.on('message', function(value) { channel.port2.postMessage(value.toUpperCase()); channel.port2.close(); wt.parentPort.close(); }); wt.parentPort.postMessage({ port: channel.port1 }, [channel.port1]);\"; var worker = new Worker(source, { eval: true }); var exit = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); var transferred = await new Promise(function(resolve) { worker.once('message', function(value) { resolve(value.port); }); }); var response = new Promise(function(resolve) { transferred.once('message', resolve); }); transferred.postMessage('worker-port'); var value = await response, code = await exit; transferred.close(); return [value, code]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_native_outbound_port_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"["WORKER-PORT",0]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_failed_clone_keeps_transfer_endpoints_attached() {
        let dir = temp_registry("builtin_worker_native_failed_transfer");
    fs::write(
        dir.join("index.js"),
        "var wt = require('node:worker_threads'); module.exports = async function () { var worker = new wt.Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); var channel = new wt.MessageChannel(), buffer = new ArrayBuffer(3), reads = 0, thrown = new Error('getter failed'), same = false; var value = { port: channel.port1, get fail() { reads++; throw thrown; } }; try { worker.postMessage(value, [channel.port1, buffer]); } catch (error) { same = error === thrown; } var attached = !channel.port1.__thawClosed && channel.port1.__thawPeer === channel.port2 && channel.port2.__thawPeer === channel.port1 && buffer.byteLength === 3, delivered = new Promise(function(resolve) { channel.port1.once('message', resolve); }); channel.port2.postMessage('still-connected'); var ping = await delivered; await worker.terminate(); channel.port1.close(); channel.port2.close(); return [same, reads, attached, ping]; };",
    )
    .unwrap();
    let node_modules = temp_registry("builtin_worker_native_failed_transfer_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"[true,1,true,"still-connected"]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_reentrant_transfer_invalidation_is_rejected_before_delivery() {
    let dir = temp_registry("builtin_worker_native_reentrant_transfer");
    fs::write(
        dir.join("index.js"),
        "var wt = require('node:worker_threads'); module.exports = async function () { var worker = new wt.Worker(\"var wt = require('node:worker_threads'); wt.parentPort.on('message', function(value) { wt.parentPort.postMessage(value === 'barrier' ? 'barrier' : 'unexpected'); });\", { eval: true }), unexpected = 0, barrier; await new Promise(function(resolve, reject) { worker.once('online', resolve); worker.once('error', reject); }); var drained = new Promise(function(resolve) { worker.on('message', function(value) { if (value === 'barrier') resolve(); else unexpected++; }); }); var first = new wt.MessageChannel(), second = new wt.MessageChannel(), secondClosed = false, value = { first: first.port1, get invalidate() { second.port1.close(); return true; } }, threw = false; second.port1.once('close', function() { secondClosed = true; }); try { worker.postMessage(value, [first.port1, second.port1]); } catch (_) { threw = true; } worker.postMessage('barrier'); await drained; var getterEffect = secondClosed; await worker.terminate(); first.port1.close(); first.port2.close(); second.port2.close(); return [threw, unexpected, getterEffect]; };",
    )
    .unwrap();
    let node_modules = temp_registry("builtin_worker_native_reentrant_transfer_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"[false,1,true]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}


#[test]
fn worker_threads_child_direct_invalid_transfer_does_not_pin_worker_exit() {
    let dir = temp_registry("builtin_worker_child_direct_invalid_transfer");
    fs::write(
        dir.join("index.js"),
        "var wt = require('node:worker_threads'); module.exports = async function () { var delivered = 0, listener = function() { delivered++; }; process.on('workerMessage', listener); var source = \"var wt = require('node:worker_threads'), first = new wt.MessageChannel(), second = new wt.MessageChannel(); wt.postMessageToThread(0, { first: first.port1, get invalidate() { second.port1.close(); return true; } }, [first.port1, second.port1]).then(function() { wt.parentPort.postMessage('unexpected'); wt.parentPort.close(); }, function() { first.port1.close(); first.port2.close(); second.port2.close(); wt.parentPort.postMessage('rejected'); });\", worker = new wt.Worker(source, { eval: true }), exited = new Promise(function(resolve) { worker.once('exit', resolve); }), message = await new Promise(function(resolve, reject) { worker.once('message', resolve); worker.once('error', reject); }), code = await exited; process.off('workerMessage', listener); return [message, code, delivered]; };",
    )
    .unwrap();
    let node_modules = temp_registry("builtin_worker_child_direct_invalid_transfer_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"["rejected",0,0]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_direct_failed_clone_keeps_buffer_attached() {
        let dir = temp_registry("builtin_worker_native_direct_failed_transfer");
    fs::write(
        dir.join("index.js"),
        "var wt = require('node:worker_threads'); module.exports = async function () { var worker = new wt.Worker(\"process.on('workerMessage', function() {});\", { eval: true }); await new Promise(function(resolve, reject) { worker.once('online', resolve); worker.once('error', reject); }); var buffer = new ArrayBuffer(2), reads = 0, thrown = new Error('getter failed'), same = false, value = { get fail() { reads++; throw thrown; } }; try { await wt.postMessageToThread(worker.threadId, value, [buffer]); } catch (error) { same = error === thrown; } var length = buffer.byteLength; await worker.terminate(); return [same, reads, length]; };",
    )
    .unwrap();
    let node_modules = temp_registry("builtin_worker_native_direct_failed_transfer_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"[true,1,2]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_transfers_native_ports_between_workers_in_both_directions() {
    let dir = temp_registry("builtin_worker_native_port_forward");
    fs::write(
        dir.join("index.js"),
        "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var sourceA = \"var wt = require('node:worker_threads'), channel = new wt.MessageChannel(); channel.port2.on('message', function(value) { channel.port2.postMessage(value === 'ping' ? 'pong' : value); channel.port2.close(); wt.parentPort.close(); }); wt.parentPort.postMessage({ port: channel.port1 }, [channel.port1]);\"; var sourceB = \"var wt = require('node:worker_threads'); wt.parentPort.once('message', function(value) { var port = value.port; port.on('message', function(message) { wt.parentPort.postMessage(message); port.close(); wt.parentPort.close(); }); port.postMessage('ping'); });\"; var a = new Worker(sourceA, { eval: true }), b = new Worker(sourceB, { eval: true }), exits = Promise.all([new Promise(function(resolve) { a.once('exit', resolve); }), new Promise(function(resolve) { b.once('exit', resolve); })]); var port = await new Promise(function(resolve, reject) { a.once('message', function(value) { resolve(value.port); }); a.once('error', reject); }); var result = new Promise(function(resolve, reject) { b.once('message', resolve); b.once('error', reject); }); b.postMessage({ port: port }, [port]); var value = await result; await exits; return [value]; };",
    )
    .unwrap();
    let node_modules = temp_registry("builtin_worker_native_port_forward_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"["pong"]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_redirects_stdin_stdout_and_stderr() {
    let dir = temp_registry("builtin_worker_stdio");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = \"var wt = require('node:worker_threads'); process.stdin.setEncoding('utf8'); process.stdin.on('data', function(value) { console.log('stdout', value); console.error('stderr', value); wt.parentPort.postMessage(value.toUpperCase()); wt.parentPort.close(); });\"; var worker = new Worker(source, { eval: true, stdin: true, stdout: true, stderr: true }); worker.stdout.setEncoding('utf8'); worker.stderr.setEncoding('utf8'); var output = '', errors = '', message; worker.stdout.on('data', function(value) { output += value; }); worker.stderr.on('data', function(value) { errors += value; }); worker.on('message', function(value) { message = value; }); var exited = new Promise(function(resolve, reject) { worker.on('error', reject); worker.on('exit', resolve); }); worker.stdin.end('hello'); var code = await exited; return [message, output, errors, code, worker.stdin.writableFinished, worker.stdout.readableEnded, worker.stderr.readableEnded]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_stdio_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"["HELLO","stdout hello\n","stderr hello\n",0,true,true,true]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}


#[test]
fn worker_terminate_resolves_with_one_shared_exit_code() {
    let dir = temp_registry("builtin_worker_terminate");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var worker = new Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); var exits = []; worker.on('exit', function(code) { exits.push(code); }); var first = worker.terminate(); var second = worker.terminate(); var code = await first; return [first === second, code, await second, await worker.terminate(), exits, Symbol.asyncDispose ? worker[Symbol.asyncDispose] === worker.terminate : true]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_terminate_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    // Node 22.23.1: a fresh promise per terminate() call; the worker exits 0 (terminated before it started)
    let result = run_js_json(&bundle);
    assert_eq!(result, "[false,0,0,null,[0],false]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}



#[test]
fn worker_process_exit_and_chdir_are_worker_local() {
    let dir = temp_registry("builtin_worker_process_exit");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var cwd = process.cwd(); var worker = new Worker(\"var wt = require('node:worker_threads'); process.on('exit', function(c) { wt.parentPort.postMessage('exit-handler:' + c); }); var r; try { process.chdir('/'); } catch (e) { r = [e.name, e.code, e.message]; } wt.parentPort.postMessage(r); setTimeout(function() { wt.parentPort.postMessage('never'); }, 50); process.exit(7); wt.parentPort.postMessage('after');\", { eval: true }); var messages = []; worker.on('message', function(m) { messages.push(m); }); var code = await new Promise(function(resolve) { worker.on('exit', resolve); }); return [code, messages, process.cwd() === cwd]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_process_exit_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    // Verified against node 22.22.2: the exit code is the Worker's, chdir is
    // unsupported, the timer scheduled before exit never fires, and the parent
    // process (this test) survives.
    assert_eq!(
        result,
        r#"[7,[["TypeError","ERR_WORKER_UNSUPPORTED_OPERATION","process.chdir() is not supported in workers"],"exit-handler:7"],true]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

// Node-valid form: startCpuProfile needs a name, and profile.stop() resolves a JSON string.
#[test]
fn worker_diagnostics_follow_running_state() {
    let dir = temp_registry("builtin_worker_diagnostics");
    fs::write(
        dir.join("index.js"),
        "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var worker = new Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); await new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); var cpu = await worker.cpuUsage(); var heap = await worker.getHeapStatistics(); var snapshot = await worker.getHeapSnapshot(); snapshot.setEncoding('utf8'); var text = ''; await new Promise(function(resolve, reject) { snapshot.on('data', function(chunk) { text += chunk; }); snapshot.on('end', resolve); snapshot.on('error', reject); }); var graph = JSON.parse(text), count = graph.snapshot.node_count; var profile = await worker.startCpuProfile('p'); var stopped = JSON.parse(await profile.stop()); var ids = stopped.nodes.map(function(node) { return node.id; }); await worker.terminate(); var stoppedError; try { await worker.cpuUsage(); } catch (error) { stoppedError = error.code; } return [Number.isInteger(cpu.user) && cpu.user >= 0 && Number.isInteger(cpu.system) && cpu.system >= 0, heap.number_of_native_contexts, count > 0 && graph.nodes.length === 7 * count && graph.edges.length === 3 * graph.snapshot.edge_count && Object.keys(graph).join(',') === 'snapshot,nodes,edges,trace_function_infos,trace_tree,samples,locations,strings', stopped.nodes.length >= 1, stopped.samples.length >= 1, stopped.timeDeltas.length === stopped.samples.length, stopped.nodes[0].callFrame.functionName === '(root)', stopped.samples.every(function(id) { return ids.indexOf(id) !== -1; }), stopped.endTime >= stopped.startTime, stoppedError]; };",
    )
    .unwrap();
    let node_modules = temp_registry("builtin_worker_diagnostics_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    assert_eq!(
        run_js(&bundle),
        r#"[true,1,true,true,true,true,true,true,true,"ERR_WORKER_NOT_RUNNING"]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_routes_messages_by_thread_id() {
    let dir = temp_registry("builtin_worker_direct_messages");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); } module.exports = async function () { var fromWorker = []; function mainListener(value, source) { fromWorker.push([value.kind, source]); } process.on('workerMessage', mainListener); var source = \"var wt = require('node:worker_threads'); wt.parentPort.on('message', function() {}); process.on('workerMessage', function(value, source) { wt.parentPort.postMessage(['direct:' + value.kind, source]); wt.parentPort.close(); }); wt.postMessageToThread(0, { kind: 'hello' }).then(function() { wt.parentPort.postMessage(['ready', wt.threadId]); });\"; var worker = new workers.Worker(source, { eval: true }); var messages = []; var direct; var completed = new Promise(function(resolve, reject) { worker.on('message', function(value) { messages.push(value); if (value[0] === 'ready') workers.postMessageToThread(worker.threadId, { kind: 'ping' }).catch(reject); else direct = value; }); worker.on('error', reject); worker.on('exit', resolve); }); var exitCode = await completed; process.off('workerMessage', mainListener); var same, missing; try { await workers.postMessageToThread(0, 'x'); } catch (error) { same = error.code; } try { await workers.postMessageToThread(999999, 'x'); } catch (error) { missing = error.code; } var silent = new workers.Worker(\"require('node:worker_threads').parentPort.on('message', function() {});\", { eval: true }); await online(silent); var noListener; try { await workers.postMessageToThread(silent.threadId, 'x'); } catch (error) { noListener = error.code; } await silent.terminate(); var throwing = new workers.Worker(\"var wt = require('node:worker_threads'); wt.parentPort.on('message', function() {}); process.on('workerMessage', function() { throw new Error('listener failed'); });\", { eval: true }); await online(throwing); var listenerError; try { await workers.postMessageToThread(throwing.threadId, 'x'); } catch (error) { listenerError = [error.code, error.cause === undefined]; } await throwing.terminate(); return [fromWorker, direct, exitCode, same, missing, noListener, listenerError]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_direct_messages_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    assert_eq!(
        run_js(&bundle),
        r#"[[["hello",1]],["direct:ping",0],0,"ERR_WORKER_MESSAGING_SAME_THREAD","ERR_WORKER_MESSAGING_FAILED","ERR_WORKER_MESSAGING_FAILED",["ERR_WORKER_MESSAGING_ERRORED",true]]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

// worker.getHeapSnapshot() on thaw-js: a Readable of Node's .heapsnapshot JSON for the worker's own graph, streamed
// from its interrupt check. Rejects ERR_WORKER_NOT_RUNNING before 'online' and after exit; a terminate mid-stream
// settles (no hang).
#[test]
fn worker_get_heap_snapshot_streams_the_graph_and_follows_running_state() {
    let dir = temp_registry("builtin_worker_get_heap_snapshot");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
function idle() { return new Worker("require('node:worker_threads').parentPort.on('message', function() {});", { eval: true }); }
function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); }
function readAll(stream) { stream.setEncoding('utf8'); var text = ''; return new Promise(function(resolve, reject) { stream.on('data', function(chunk) { text += chunk; }); stream.on('end', function() { resolve(text); }); stream.on('error', reject); }); }
module.exports = async function () {
  var worker = new Worker("var keep = []; for (var i = 0; i < 4000; i++) keep.push({ i: i, label: 'n' + i }); require('node:worker_threads').parentPort.on('message', function() {});", { eval: true });
  var early; try { await worker.getHeapSnapshot(); early = 'resolved'; } catch (error) { early = error.code; }
  await online(worker);
  var text = await readAll(await worker.getHeapSnapshot());
  var graph = JSON.parse(text);
  var count = graph.snapshot.node_count, edges = graph.snapshot.edge_count;
  await worker.terminate();
  var stopped; try { await worker.getHeapSnapshot(); stopped = 'resolved'; } catch (error) { stopped = error.code; }
  var middle = idle();
  await online(middle);
  var cut = await middle.getHeapSnapshot();
  var firstChunk = new Promise(function(resolve) { cut.once('data', function() { resolve('chunk'); }); });
  var first = await firstChunk;
  await middle.terminate();
  return [early, Object.keys(graph).join(','), count > 0 && graph.nodes.length === 7 * count && graph.edges.length === 3 * edges, text.length > 65536, stopped, first];
};"#,
    )
    .unwrap();
    let node_modules = temp_registry("builtin_worker_get_heap_snapshot_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    assert_eq!(
        run_js(&bundle),
        r#"["ERR_WORKER_NOT_RUNNING","snapshot,nodes,edges,trace_function_infos,trace_tree,samples,locations,strings",true,true,"ERR_WORKER_NOT_RUNNING","chunk"]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_route_direct_messages_between_native_workers() {
    let dir = temp_registry("builtin_worker_direct_between_workers");
    fs::write(
            dir.join("index.js"),
            "var workers = require('node:worker_threads'); function online(worker) { return new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); } module.exports = async function () { var receiver = new workers.Worker(\"var wt = require('node:worker_threads'); wt.parentPort.on('message', function() {}); process.on('workerMessage', function(value, source) { wt.parentPort.postMessage([value.answer, source]); wt.parentPort.close(); });\", { eval: true }); await online(receiver); var sender = new workers.Worker(\"var wt = require('node:worker_threads'); wt.postMessageToThread(wt.workerData.target, { answer: 42 }).then(function() { wt.parentPort.postMessage('sent'); wt.parentPort.close(); });\", { eval: true, workerData: { target: receiver.threadId } }); var received = new Promise(function(resolve, reject) { receiver.on('message', resolve); receiver.on('error', reject); }); var sent = new Promise(function(resolve, reject) { sender.on('message', resolve); sender.on('error', reject); }); var receiverExit = new Promise(function(resolve) { receiver.on('exit', resolve); }), senderExit = new Promise(function(resolve) { sender.on('exit', resolve); }); var values = await Promise.all([received, sent, receiverExit, senderExit]); return values; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_direct_between_workers_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    assert_eq!(run_js(&bundle), r#"[[42,2],"sent",0,0]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_data_url_worker_decodes_javascript() {
    let dir = temp_registry("builtin_worker_data_url");
    fs::write(
        dir.join("index.js"),
        r#"var workers = require('node:worker_threads'); module.exports = async function () { var parentThread = workers.threadId; var source = "import * as wt from 'node:worker_threads'; wt.parentPort.postMessage({ value: wt.workerData.value, main: wt.isMainThread, thread: wt.threadId }); wt.parentPort.close();"; var worker = new workers.Worker(new URL('data:text/javascript,' + encodeURIComponent(source)), { workerData: { value: 17 } }); var events = []; await new Promise(function(resolve, reject) { worker.on('online', function() { events.push('online'); }); worker.on('message', function(value) { events.push('message:' + value.value + ':' + value.main + ':' + (value.thread !== parentThread)); }); worker.on('error', reject); worker.on('exit', function(code) { events.push('exit:' + code); resolve(); }); }); var bad = new workers.Worker(new URL('data:text/plain,not-javascript')); var badCode = await new Promise(function(resolve) { bad.on('error', function(error) { resolve(error.code); }); }); return [events, badCode]; };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_worker_data_url_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    assert_eq!(
        run_js(&bundle),
        r#"[["online","message:17:false:true","exit:0"],"ERR_UNKNOWN_MODULE_FORMAT"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_threads_load_runtime_computed_absolute_paths() {
    let dir = worker_fixture(
        "worker_runtime_absolute_paths",
        &[
            (
                "package.json",
                r##"{"imports":{"#worker-tool":{"node":"./internal/tool.cjs","default":"./internal/wrong.cjs"},"#features/*":{"require":"./internal/features/*.js"},"#escape":"../outside.js"}}"##,
            ),
            ("internal/tool.cjs", "module.exports = 11;"),
            ("internal/features/math.js", "module.exports = 12;"),
            (
                "workers/nested/dynamic-worker.js",
                "var wt = require('node:worker_threads'); var hiddenCode; try { require('runtime-worker-dependency/hidden'); } catch (error) { hiddenCode = error.code; } var missingImportCode; try { require('#missing'); } catch (error) { missingImportCode = error.code; } var exportEscapeCode; try { require('runtime-worker-dependency/escape'); } catch (error) { exportEscapeCode = error.code; } var importEscapeCode; try { require('#escape'); } catch (error) { importEscapeCode = error.code; } wt.parentPort.postMessage([wt.workerData, __filename, __dirname, require('./worker-value'), require('runtime-worker-dependency'), require('runtime-worker-dependency/feature'), require('runtime-worker-dependency/features/math'), hiddenCode, require('#worker-tool'), require('#features/math'), missingImportCode, exportEscapeCode, importEscapeCode]); wt.parentPort.close();",
            ),
            ("workers/nested/worker-value.js", "module.exports = require('./worker-package') + 1;"),
            ("workers/nested/worker-package/package.json", r#"{"main":"lib/value"}"#),
            ("workers/nested/worker-package/lib/value.js", "module.exports = require('../worker-data.json').value;"),
            ("workers/nested/worker-package/worker-data.json", r#"{"value":41}"#),
            (
                "node_modules/runtime-worker-dependency/package.json",
                r#"{"main":"wrong.cjs","exports":{".":{"node":"./entry.cjs","default":"./wrong.cjs"},"./feature":{"require":"./feature.js"},"./features/*":{"require":"./dist/*.cjs"},"./escape":"../outside.js"}}"#,
            ),
            ("node_modules/runtime-worker-dependency/entry.cjs", "module.exports = 8;"),
            ("node_modules/runtime-worker-dependency/feature.js", "module.exports = 9;"),
            ("node_modules/runtime-worker-dependency/dist/math.cjs", "module.exports = 10;"),
        ],
    );
    let worker_dir = dir.join("workers/nested");
    let worker_path = worker_dir.join("dynamic-worker.js");
    let path = serde_json::to_string(worker_path.to_str().unwrap()).unwrap();
    // Worker message plus exit code, as the QuickJS version returned them (its typeof probes are dropped).
    let result = run_worker_fixture(
        &dir,
        &format!(
            "var worker = new Worker({path}, {{ workerData: 23 }}), out = {{ messages: [], errors: [] }}; \
             return new Promise(function (resolve) {{ \
                 worker.on('message', function (value) {{ out.messages.push(value); }}); \
                 worker.on('error', function (error) {{ out.errors.push(error.code || error.name); }}); \
                 worker.on('exit', function (code) {{ resolve([out.messages[0], code, out.errors]); }}); \
             }});"
        ),
    );
    let expected = serde_json::json!([
        [
            23,
            worker_path.to_string_lossy(),
            worker_dir.to_string_lossy(),
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
        []
    ])
    .to_string();
    assert_eq!(result, expected);
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn process_default_stdio_preserves_raw_buffer_bytes() {
    use std::process::Command;
    const CHILD: &str = "THAW_CAPTURE_PROCESS_STREAM_TEST";
    if std::env::var_os(CHILD).is_some() {
        let dir = temp_registry("process_default_stdio_raw_bytes");
        fs::write(
            dir.join("index.js"),
            r#"module.exports = async function () { var callbacks = 0, invalidCode = '', backing = Buffer.from([90, 0, 255, 128, 195, 40, 91]); process.stdout.write(Buffer.from([250])); var out = process.stdout.write(backing.subarray(1, 6), function(error) { if (error) throw error; callbacks++; }); process.stdout.write(Buffer.from([251])); var err = process.stderr.write('05ff80c328', 'hex', function(error) { if (error) throw error; callbacks++; }); try { process.stdout.write(1); } catch (error) { invalidCode = error.code; } await new Promise(function(resolve) { queueMicrotask(resolve); }); return [out, err, invalidCode, callbacks]; };"#,
        )
        .unwrap();
        let node_modules = temp_registry("process_default_stdio_raw_bytes_node_modules");
        let (bundle, _, _, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(run_js(&bundle), r#"[true,true,"ERR_INVALID_ARG_TYPE",2]"#);
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&node_modules);

        let dir = temp_registry("process_default_stdio_worker_raw_bytes");
        fs::write(
            dir.join("index.js"),
            r#"var Worker = require('node:worker_threads').Worker; module.exports = async function () { async function run(out, err, capture) { var source = "var wt = require('node:worker_threads'); process.stdout.write(Buffer.from([" + out + ", 255, 128, 195, 40])); process.stderr.write(Buffer.from([" + err + ", 255, 128, 195, 40])); wt.parentPort.close();"; var worker = new Worker(source, { eval: true, stdout: !!capture, stderr: !!capture }), waits = []; if (capture) { waits.push(new Promise(function(resolve) { worker.stdout.on('data', function(chunk) { process.stdout.write(chunk); }); worker.stdout.on('end', resolve); })); waits.push(new Promise(function(resolve) { worker.stderr.on('data', function(chunk) { process.stderr.write(chunk); }); worker.stderr.on('end', resolve); })); } waits.push(new Promise(function(resolve) { worker.on('exit', resolve); })); var values = await Promise.all(waits); return values[values.length - 1]; } return [await run(16, 21, false), await run(48, 53, true)]; };"#,
        )
        .unwrap();
        let node_modules = temp_registry("process_default_stdio_worker_raw_bytes_node_modules");
        let (bundle, _, file_count, _) =
            bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
        assert_eq!(file_count, 4);
        assert_eq!(run_js(&bundle), "[0,0]");
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_dir_all(node_modules);
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "tests::process_default_stdio_preserves_raw_buffer_bytes",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "child test failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.windows(7).any(|bytes| bytes == [250, 0, 255, 128, 195, 40, 251]));
    assert!(output.stderr.windows(5).any(|bytes| bytes == [5, 255, 128, 195, 40]));
    assert!(output.stdout.windows(5).any(|bytes| bytes == [16, 255, 128, 195, 40]));
    assert!(output.stderr.windows(5).any(|bytes| bytes == [21, 255, 128, 195, 40]));
    assert!(output.stdout.windows(5).any(|bytes| bytes == [48, 255, 128, 195, 40]));
    assert!(output.stderr.windows(5).any(|bytes| bytes == [53, 255, 128, 195, 40]));
}

#[test]
fn worker_threads_native_stdio_defaults_to_parent_and_capture_stays_separate() {
    let dir = temp_registry("builtin_worker_default_stdio");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker; module.exports = async function () { var output = '', errors = '', capturedOutput = '', capturedErrors = '', originalOut = process.stdout.write, originalErr = process.stderr.write, originalSpawn = globalThis.__thaw_worker_spawn; process.stdout.write = function(value, encoding, callback) { if (typeof encoding === 'function') callback = encoding; output += Buffer.from(value).toString('hex'); if (callback) queueMicrotask(callback); return true; }; process.stderr.write = function(value, encoding, callback) { if (typeof encoding === 'function') callback = encoding; errors += Buffer.from(value).toString('hex'); if (callback) queueMicrotask(callback); return true; }; var source = "var wt = require('node:worker_threads'), out = Buffer.from([99, 0, 255, 128, 100]); process.stdout.write(out.subarray(1, 4)); process.stderr.write('01fe81', 'hex'); wt.parentPort.close();"; try { var worker = new Worker(source, { eval: true }); var defaultExit = new Promise(function(resolve) { worker.on('exit', resolve); }); await defaultExit; var captured = new Worker("var wt = require('node:worker_threads'), out = new Uint8Array([99, 2, 253, 130, 100]); process.stdout.write(new DataView(out.buffer, 1, 3)); process.stderr.write('03fc83', 'hex'); wt.parentPort.close();", { eval: true, stdout: true, stderr: true }); captured.stdout.on('data', function(value) { capturedOutput += Buffer.from(value).toString('hex'); }); captured.stderr.on('data', function(value) { capturedErrors += Buffer.from(value).toString('hex'); }); var captureExit = new Promise(function(resolve) { captured.on('exit', resolve); }); await captureExit; return [worker.stdout === null, worker.stderr === null, output, errors, capturedOutput, capturedErrors, captured.stdout.readableEnded, captured.stderr.readableEnded]; } finally { globalThis.__thaw_worker_spawn = originalSpawn; process.stdout.write = originalOut; process.stderr.write = originalErr; } };"#,
    )
    .unwrap();
    let node_modules = temp_registry("builtin_worker_default_stdio_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"[false,false,"00ff80","01fe81","02fd82","03fc83",true,true]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_stdin_transports_bytes_and_end_in_order() {
    let dir = temp_registry("builtin_worker_stdin_bytes");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker; module.exports = async function () { async function run(native) { var originalSpawn = globalThis.__thaw_worker_spawn; if (!native) globalThis.__thaw_worker_spawn = undefined; try { var source = "var wt = require('node:worker_threads'), received = []; process.stdin.on('data', function(chunk) { received.push([Buffer.isBuffer(chunk), Buffer.from(chunk).toString('hex')]); }); process.stdin.on('end', function() { wt.parentPort.postMessage(received); wt.parentPort.close(); });"; var worker = new Worker(source, { eval: true, stdin: true }), writes = 0, ends = 0, backing = Buffer.from([99, 0, 255, 88]), finalBacking = Buffer.from([77, 128, 66]); await new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); var messagesPromise = new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); }), exitPromise = new Promise(function(resolve) { worker.on('exit', resolve); }); worker.stdin.write(backing.subarray(1, 3), function(error) { if (error) throw error; writes++; }); worker.stdin.write('Aÿ', 'latin1', function(error) { if (error) throw error; writes++; }); worker.stdin.end(finalBacking.subarray(1, 2), function(error) { if (error) throw error; ends++; }); var messages = await messagesPromise, code = await exitPromise; return [code, writes, ends, messages]; } finally { globalThis.__thaw_worker_spawn = originalSpawn; } } return [await run(true)]; };"#,
    )
    .unwrap();
    let node_modules = temp_registry("builtin_worker_stdin_bytes_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"[[0,2,1,[[true,"00ff"],[true,"41ff"],[true,"80"]]]]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_stdin_reports_host_failures_once() {
    let dir = temp_registry("builtin_worker_stdin_host_failure");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker; module.exports = async function () { async function run(throws) { var original = globalThis.__thaw_worker_stdin, worker = new Worker("require('node:worker_threads').parentPort.on('message', function() {});", { eval: true, stdin: true }), calls = 0, errorEvents = 0; await new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); var streamError = new Promise(function(resolve) { worker.stdin.on('error', function(error) { errorEvents++; resolve(error && error.message || ''); }); }); globalThis.__thaw_worker_stdin = function() { if (throws) throw new Error('host stdin failure'); return false; }; var result = await new Promise(function(resolve) { worker.stdin.write(Buffer.from([1]), function(error) { calls++; resolve([error && error.code || '', error && error.message || '']); }); }); var emitted = await streamError; globalThis.__thaw_worker_stdin = original; var exited = new Promise(function(resolve) { worker.on('exit', resolve); }); await worker.terminate(); var code = await exited; return [calls, result, errorEvents, emitted, code]; } return [await run(false), await run(true)]; };"#,
    )
    .unwrap();
    let node_modules = temp_registry("builtin_worker_stdin_host_failure_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"[[1,["ERR_WORKER_NOT_RUNNING","Worker stdin is not available"],1,"Worker stdin is not available",1],[1,["","host stdin failure"],1,"host stdin failure",1]]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_stdin_string_decoder_handles_split_utf8() {
    let dir = temp_registry("builtin_worker_stdin_string_decoder");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker; module.exports = async function () { async function run(incomplete) { var source = "var wt = require('node:worker_threads'), events = []; process.stdin.setEncoding('utf8'); process.stdin.on('data', function(chunk) { events.push(chunk); }); process.stdin.on('end', function() { events.push('end'); wt.parentPort.postMessage([events, process.stdin.encoding]); wt.parentPort.close(); });"; var worker = new Worker(source, { eval: true, stdin: true }); await new Promise(function(resolve, reject) { worker.on('online', resolve); worker.on('error', reject); }); var message = new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); }), exit = new Promise(function(resolve) { worker.on('exit', resolve); }); worker.stdin.write(Buffer.from([226, 130])); if (incomplete) worker.stdin.end(); else worker.stdin.end(Buffer.from([172])); var result = await message, code = await exit; return [code, result]; } return [await run(false), await run(true)]; };"#,
    )
    .unwrap();
    let node_modules = temp_registry("builtin_worker_stdin_string_decoder_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"[[0,[["€","end"],null]],[0,[["�","end"],null]]]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_runtime_round_trips_parent_messages() {
    let dir = temp_registry("builtin_worker_native_round_trip");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = \"var wt = require('node:worker_threads'); wt.parentPort.on('message', function(value) { wt.parentPort.postMessage([value * wt.workerData, wt.threadId]); wt.parentPort.close(); });\"; var parentThread = require('node:worker_threads').threadId; var worker = new Worker(source, { eval: true, workerData: 7 }); var exit = new Promise(function(resolve) { worker.on('exit', resolve); }); var result = await new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); worker.postMessage(6); }); var code = await exit; return [result[0], result[1] !== parentThread, code]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_native_round_trip_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"[42,true,0]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_ref_unreffed_workers_follow_host_liveness() {
    let dir = temp_registry("builtin_worker_native_ref");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker; module.exports = async function () { var source = "var wt = require('node:worker_threads'); wt.parentPort.on('message', function(value) { wt.parentPort.postMessage(value); wt.parentPort.close(); });"; var worker = new Worker(source, { eval: true }); var unrefReturn = worker.unref(); var sibling = new Worker(source, { eval: true }); var refReturn = worker.ref(); worker.unref(); var exits = [new Promise(function(resolve) { worker.on('exit', resolve); }), new Promise(function(resolve) { sibling.on('exit', resolve); })]; var workerMessage = new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); }), siblingMessage = new Promise(function(resolve, reject) { sibling.on('message', resolve); sibling.on('error', reject); }); worker.postMessage('unref'); var messages = [await workerMessage]; sibling.postMessage('refed'); messages.push(await siblingMessage); var codes = await Promise.all(exits); return [unrefReturn === undefined, refReturn === undefined, messages, codes]; };"#,
    )
    .unwrap();
    let node_modules = temp_registry("builtin_worker_native_ref_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    // Node 22: Worker.ref() and Worker.unref() return undefined (Worker has no hasRef).
    assert_eq!(
        run_js(&bundle),
        r#"[true,true,["unref","refed"],[0,0]]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn worker_threads_native_runtime_transfers_structured_array_buffers() {
    let dir = temp_registry("builtin_worker_native_structured_transfer");
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var first = new ArrayBuffer(3), second = new ArrayBuffer(2); new Uint8Array(first).set([1, 2, 3]); new Uint8Array(second).set([4, 5]); var data = { buffer: first, map: new Map([['answer', 42]]) }; data.self = data; var source = \"var wt = require('node:worker_threads'); wt.parentPort.on('message', function(value) { var reply = { first: Array.from(new Uint8Array(wt.workerData.buffer)), second: Array.from(new Uint8Array(value.buffer)), answer: wt.workerData.map.get('answer'), workerCycle: wt.workerData.self === wt.workerData, messageCycle: value.self === value }; reply.self = reply; wt.parentPort.postMessage(reply); wt.parentPort.close(); });\"; var worker = new Worker(source, { eval: true, workerData: data, transferList: [first] }); var message = { buffer: second }; message.self = message; var exit = new Promise(function(resolve) { worker.on('exit', resolve); }); var reply = new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); }); worker.postMessage(message, [second]); var value = await reply, code = await exit; return [first.byteLength, second.byteLength, value.first, value.second, value.answer, value.workerCycle, value.messageCycle, value.self === value, code]; };",
        )
        .unwrap();
    let node_modules = temp_registry("builtin_worker_native_structured_transfer_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(
        result,
        r#"[0,0,[1,2,3],[4,5],42,true,true,true,0]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn bundler_embeds_static_file_url_worker_sources() {
    let dir = temp_registry("builtin_worker_file_url");
    fs::create_dir_all(dir.join("lib/sub")).unwrap();
    fs::write(
        dir.join("double.js"),
        "module.exports = function(value) { return value * 2; };",
    )
    .unwrap();
    fs::write(
            dir.join("worker.js"),
            "import { parentPort, workerData } from 'node:worker_threads'; import double from './double.js'; parentPort.postMessage(double(workerData)); parentPort.close();",
        )
        .unwrap();
    fs::write(
        dir.join("lib/worker.js"),
        "require('node:worker_threads').parentPort.postMessage(999);",
    )
    .unwrap();
    fs::write(
        dir.join("stdin-worker.js"),
        "import { parentPort } from 'node:worker_threads'; var text = ''; process.stdin.setEncoding('utf8'); process.stdin.on('data', function(value) { text += value; }); process.stdin.on('end', function() { parentPort.postMessage(text); parentPort.close(); });",
    )
    .unwrap();
    fs::write(
        dir.join("lib/entry.js"),
        "var Worker = require('node:worker_threads').Worker; var path = require('node:path'); module.exports = async function(run, events) { await run(new Worker(path.join(__dirname, '../worker.js'), { workerData: 7 }), events); await run(new Worker(path.join(__dirname, 'sub', '../../worker.js'), { workerData: 8 }), events); await run(new Worker(path.resolve(__dirname, 'sub', '../../worker.js'), { workerData: 9 }), events); await run(new Worker('../worker.js', { workerData: 10 }), events); };",
    )
    .unwrap();
    fs::write(
            dir.join("index.js"),
            "var Worker = require('node:worker_threads').Worker; var path = require('node:path'); var nested = require('./lib/entry'); const workerFile = './' + 'worker.js'; const joinedWorkerFile = path.join(__dirname, 'worker.js'); function run(worker, events) { return new Promise(function(resolve, reject) { worker.on('message', function(value) { events.push(value); }); worker.on('error', reject); worker.on('exit', function(code) { events.push(code); resolve(); }); }); } module.exports = async function () { var events = []; await run(new Worker(new URL('./worker.js', import.meta.url), { workerData: 21 }), events); await run(new Worker('./worker.js', { workerData: 11 }), events); await run(new Worker(`./${'worker'}.js`, { workerData: 5 }), events); await run(new Worker(workerFile, { workerData: 3 }), events); await run(new Worker(joinedWorkerFile, { workerData: 2 }), events); await nested(run, events); var stdinWorker = new Worker(new URL('./stdin-worker.js', import.meta.url), { stdin: true }), stdinExit = run(stdinWorker, events); await new Promise(function(resolve, reject) { stdinWorker.on('online', resolve); stdinWorker.on('error', reject); }); stdinWorker.stdin.write(Buffer.from([226, 130])); stdinWorker.stdin.end(Buffer.from([172])); await stdinExit; return events; };",
        )
        .unwrap();
    let outside = "var Worker = require('node:worker_threads').Worker; var path = require('node:path'); new Worker(path.join(__dirname, '../../outside.js'));";
    let error = rewrite_static_worker_urls(outside, &dir.join("lib/entry.js"), "pkg", &dir)
        .expect_err("a parent path escaping the package must be rejected before reading");
    assert!(error.contains("outside package"), "{error}");
    let empty_node_modules = temp_registry("builtin_worker_file_url_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 9);
    fs::remove_dir_all(&dir).unwrap();
    let result = run_js(&bundle);
    assert_eq!(result, r#"[42,0,22,0,10,0,6,0,4,0,14,0,16,0,18,0,20,0,"€",0]"#);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn bundled_worker_dynamic_import_converts_once_and_keeps_worker_module() {
    // Unrun regression: the Worker entry's dynamic import helper must use
    // the same ToString boundary while preserving its local worker_threads.
    let dir = temp_registry("worker_dynamic_import_specifier");
    let modules = temp_registry("worker_dynamic_import_specifier_modules");
    fs::write(dir.join("dep.js"), "exports.value = 42;").unwrap();
    fs::write(
        dir.join("worker.js"),
        "var wt = require('node:worker_threads'); var calls = 0, hint; \
         var spec = { [Symbol.toPrimitive]: function(value) { calls++; hint = value; return 'node:worker_threads'; } }; \
         wt.parentPort.on('message', function() { Promise.all([import(spec), import('./dep.js')]).then(function(values) { \
           wt.parentPort.postMessage([values[0].parentPort === wt.parentPort, calls, hint, values[1].value]); \
           wt.parentPort.close(); \
         }); });",
    )
    .unwrap();
    fs::write(
        dir.join("index.js"),
        "var Worker = require('node:worker_threads').Worker; \
         module.exports = function() { return new Promise(function(resolve, reject) { \
           var worker = new Worker(new URL('./worker.js', import.meta.url)), result; \
           worker.on('message', function(value) { result = value; }); \
           worker.on('error', reject); worker.on('exit', function(code) { resolve([result, code]); }); \
           worker.postMessage('go'); \
         }); };",
    )
    .unwrap();
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&modules, "pkg", &dir, "index.js").unwrap();
    assert!(file_count >= 4);
    fs::remove_dir_all(&dir).unwrap();
    assert_eq!(run_js(&bundle), "[[true,1,\"string\",42],0]");
    let _ = fs::remove_dir_all(modules);
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
        let dir = temp_registry("builtin_broadcast_channel");
    fs::write(
            dir.join("index.js"),
            "var BroadcastChannel = require('node:worker_threads').BroadcastChannel; module.exports = async function () { var sender = new BroadcastChannel('room'); var first = new BroadcastChannel('room'); var second = new BroadcastChannel('room'); var isolated = new BroadcastChannel('elsewhere'); var source = { nested: { value: 4 } }; var firstMessage = new Promise(function(resolve) { first.onmessage = function(event) { event.data.nested.value = 8; resolve(event.data.nested.value); }; }); var secondMessage = new Promise(function(resolve) { second.addEventListener('message', function(event) { resolve(event.data.nested.value); }, { once: true }); }); var isolatedCalled = false; isolated.onmessage = function() { isolatedCalled = true; }; sender.postMessage(source); source.nested.value = 9; var values = await Promise.all([firstMessage, secondMessage]); first.close(); var closedError = false; try { first.postMessage('x'); } catch (error) { closedError = error.name === 'InvalidStateError'; } sender.close(); second.close(); isolated.close(); return [values[0], values[1], isolatedCalled, closedError, sender.name, sender.ref() === sender, sender.unref() === sender]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_broadcast_channel_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let result = run_js(&bundle);
    assert_eq!(result, r#"[8,4,false,true,"room",true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn worker_esm_star_diamond_uses_the_same_terminal_origin() {
    let package = temp_registry("worker-star-origin");
    let modules = temp_registry("worker-star-origin-modules");
    fs::write(package.join("index.js"), "var Worker = require('node:worker_threads').Worker; module.exports = function() { return new Promise(function(resolve, reject) { var worker = new Worker(new URL('./worker.js', import.meta.url)); worker.on('message', resolve); worker.on('error', reject); }); };").unwrap();
    fs::write(package.join("worker.js"), "import { parentPort } from 'node:worker_threads'; import * as self from './worker.js'; export * from './a.js'; export * from './b.js'; parentPort.postMessage(self.value); parentPort.close();").unwrap();
    fs::write(package.join("a.js"), "export * from './leaf.js';").unwrap();
    fs::write(package.join("b.js"), "export * from './leaf.js';").unwrap();
    fs::write(package.join("leaf.js"), "export const value = 7;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "worker-origin-pkg", &package, "index.js").unwrap();
    assert_eq!(run_js(&bundle), "7");
    let _ = fs::remove_dir_all(package);
    let _ = fs::remove_dir_all(modules);
}

#[test]
fn worker_esm_cycle_uses_one_cached_main_module_identity() {
    let package = temp_registry("worker-main-cycle-origin");
    let modules = temp_registry("worker-main-cycle-origin-modules");
    fs::write(package.join("index.js"), "var Worker = require('node:worker_threads').Worker; module.exports = function() { return new Promise(function(resolve, reject) { var worker = new Worker(new URL('./worker.js', import.meta.url)); worker.on('message', resolve); worker.on('error', reject); }); };").unwrap();
    fs::write(package.join("worker.js"), "import { parentPort } from 'node:worker_threads'; import * as self from './worker.js'; import { readMain } from './dep.js'; import { basename } from 'node:path'; import { fileURLToPath } from 'node:url'; var __thaw_finish_worker_main_0 = 'user-finish'; var __thaw_worker_main_error_0 = 'user-error'; globalThis.workerMainRuns = (globalThis.workerMainRuns || 0) + 1; export const identity = {}; parentPort.postMessage([workerMainRuns, readMain() === self.identity, basename(fileURLToPath(import.meta.url)), __thaw_finish_worker_main_0, __thaw_worker_main_error_0]); parentPort.close();").unwrap();
    fs::write(package.join("dep.js"), "import * as main from './worker.js'; export function readMain() { return main.identity; }").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "worker-origin-pkg", &package, "index.js").unwrap();
    assert_eq!(run_js(&bundle), "[1,true,\"worker.js\",\"user-finish\",\"user-error\"]");
    let _ = fs::remove_dir_all(package);
    let _ = fs::remove_dir_all(modules);
}

#[test]
fn worker_entry_throw_of_each_falsy_value_reaches_the_error_event() {
    let package = temp_registry("worker-falsy-main-ready");
    let modules = temp_registry("worker-falsy-main-ready-modules");
    fs::write(package.join("index.js"), "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var reasons = [0, false, '', null, undefined], results = []; for (var i = 0; i < reasons.length; i++) { var reason = reasons[i]; results.push(await new Promise(function (resolve) { var worker = new Worker(new URL('./worker.js', import.meta.url), { workerData: reason }); worker.on('error', function (error) { resolve(Object.is(error, reason)); }); })); } return results; };").unwrap();
    fs::write(package.join("worker.js"), "var wt = require('node:worker_threads'); throw wt.workerData;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "worker-falsy-pkg", &package, "index.js").unwrap();
    // Each falsy value thrown by the worker's entry reaches the parent's 'error' listener unchanged (Object.is).
    assert_eq!(run_js(&bundle), "[true,true,true,true,true]");
    let _ = fs::remove_dir_all(package);
    let _ = fs::remove_dir_all(modules);
}

#[test]
fn worker_main_ready_rejects_every_falsy_thrown_value() {
    let dir = worker_fixture("worker_main_ready_falsy", &[]);
    let result = run_worker_fixture(
        &dir,
        r#"var out = [];
        for (var source of ['throw 0', 'throw false', 'throw ""', 'throw null', 'throw undefined']) {
            out.push(await new Promise(function (resolve) {
                var worker = new Worker(source, { eval: true });
                worker.on('error', function (error) { resolve(['err', typeof error, String(error)]); });
                worker.on('exit', function (code) { resolve(['exit', code]); });
            }));
        }
        return out;"#,
    );
    assert_eq!(
        result,
        r#"[["err","number","0"],["err","boolean","false"],["err","string",""],["err","object","null"],["err","undefined","undefined"]]"#
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn bundled_worker_entry_require_resolve_does_not_load_target() {
    let package = temp_registry("worker_entry_require_resolve");
    let modules = temp_registry("worker_entry_require_resolve_modules");
    fs::write(package.join("index.js"), "var Worker = require('node:worker_threads').Worker; module.exports = function() { return new Promise(function(resolve, reject) { var worker = new Worker(new URL('./worker.js', import.meta.url)); worker.on('message', resolve); worker.on('error', reject); }); };").unwrap();
    fs::write(package.join("worker.js"), "var wt = require('node:worker_threads'); var present = require.resolve('./dep.js'), missing; try { require.resolve('./absent.js'); } catch (error) { missing = error.code; } wt.parentPort.postMessage([present, missing, globalThis.workerResolveDepRuns || 0]); wt.parentPort.close();").unwrap();
    fs::write(package.join("dep.js"), "globalThis.workerResolveDepRuns = (globalThis.workerResolveDepRuns || 0) + 1;").unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &package, "index.js").unwrap();
    assert_eq!(run_js(&bundle), "[\"/thaw_modules/pkg/dep.js\",\"MODULE_NOT_FOUND\",0]");
    let _ = fs::remove_dir_all(package);
    let _ = fs::remove_dir_all(modules);
}

#[test]
fn worker_saved_url_keeps_bundle_affinity_for_worker_entry() {
    let dir = worker_fixture(
        "worker_saved_url_affinity",
        &[
            ("worker.js", "var parentPort = require('node:worker_threads').parentPort; var direct, created; try { require('affinity-missing'); } catch (error) { direct = error.code; } var makeRequire = require('node:module').createRequire(__filename); try { makeRequire('affinity-missing'); } catch (error) { created = error.code; } parentPort.postMessage([require('./dep.js').value, require('affinity-shared').value, direct, created]); parentPort.close();"),
            ("dep.js", "exports.value = 'A';"),
            ("node_modules/affinity-shared/package.json", r#"{"name":"affinity-shared","main":"index.js"}"#),
            ("node_modules/affinity-shared/index.js", "exports.value = 'A-bare';"),
        ],
    );
    let result = run_worker_fixture(
        &dir,
        r#"var saved, real = Worker;
        Worker = function (url) { saved = url; };
        try { new Worker(new URL('./worker.js', import.meta.url)); } finally { Worker = real; }
        var run = await runWorker(saved);
        return [run.messages, run.errors];"#,
    );
    assert_eq!(
        result,
        r#"[[["A","A-bare","MODULE_NOT_FOUND","MODULE_NOT_FOUND"]],[]]"#
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn bundled_worker_nested_spawn_resolves_from_its_own_package_directory() {
    // Node 22.23.1 (real files, pathToFileURL, 3 runs): [["A","A-bare","MODULE_NOT_FOUND","MODULE_NOT_FOUND"],["B","MODULE_NOT_FOUND"],["A",...same]]
    // A is reported by a/nested.js, started by the bundled a/worker.js; B has no node_modules, so its bare require fails.
    let dir = worker_fixture(
        "bundled_worker_nested_affinity",
        &[
            ("index.js", "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var first = await new Promise(function (resolve, reject) { var w = new Worker(new URL('./a/worker.js', import.meta.url)); w.on('message', resolve); w.on('error', reject); }); var second = await new Promise(function (resolve, reject) { var w = new Worker(new URL('./b/worker.js', import.meta.url)); w.on('message', resolve); w.on('error', reject); }); var third = await new Promise(function (resolve, reject) { var w = new Worker(new URL('./a/worker.js', import.meta.url)); w.on('message', resolve); w.on('error', reject); }); return [first, second, third]; };"),
            ("a/worker.js", "var wt = require('node:worker_threads'); var child = new wt.Worker(new URL('./nested.js', import.meta.url)); child.on('message', function (value) { wt.parentPort.postMessage(value); wt.parentPort.close(); }); child.on('error', function (error) { throw error; });"),
            ("a/nested.js", "var parentPort = require('node:worker_threads').parentPort; var direct, created; try { require('affinity-missing'); } catch (error) { direct = error.code; } var makeRequire = require('node:module').createRequire(__filename); try { makeRequire('affinity-missing'); } catch (error) { created = error.code; } parentPort.postMessage([require('./dep.js').value, require('affinity-shared').value, direct, created]); parentPort.close();"),
            ("a/dep.js", "exports.value = 'A';"),
            ("a/node_modules/affinity-shared/package.json", r#"{"name":"affinity-shared","main":"index.js"}"#),
            ("a/node_modules/affinity-shared/index.js", "exports.value = 'A-bare';"),
            ("b/worker.js", "var parentPort = require('node:worker_threads').parentPort; var bare; try { bare = require('affinity-shared').value; } catch (error) { bare = error.code; } parentPort.postMessage([require('./dep.js').value, bare]); parentPort.close();"),
            ("b/dep.js", "exports.value = 'B';"),
        ],
    );
    let modules = temp_registry("bundled_worker_nested_affinity_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(
        run_js(&bundle),
        r#"[["A","A-bare","MODULE_NOT_FOUND","MODULE_NOT_FOUND"],["B","MODULE_NOT_FOUND"],["A","A-bare","MODULE_NOT_FOUND","MODULE_NOT_FOUND"]]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(modules);
}

#[test]
fn bundled_worker_and_its_nested_worker_keep_package_directory_affinity() {
    // Node 22.23.1 (real files, pathToFileURL, 3 runs, identical): [["A","A-bare","MODULE_NOT_FOUND","MODULE_NOT_FOUND"],["B","B-bare"],["A","A-bare","MODULE_NOT_FOUND","MODULE_NOT_FOUND"]].
    // One package with two copies: a/ and b/ each own a dep.js and their own node_modules/affinity-shared; a/worker.js
    // starts a/nested.js, which resolves from its own directory, not from b/.
    let dir = worker_fixture(
        "bundled_worker_and_nested_worker_affinity",
        &[
            ("index.js", "var Worker = require('node:worker_threads').Worker; module.exports = async function () { var first = await new Promise(function (resolve, reject) { var w = new Worker(new URL('./a/worker.js', import.meta.url)); w.on('message', resolve); w.on('error', reject); }); var second = await new Promise(function (resolve, reject) { var w = new Worker(new URL('./b/worker.js', import.meta.url)); w.on('message', resolve); w.on('error', reject); }); var third = await new Promise(function (resolve, reject) { var w = new Worker(new URL('./a/worker.js', import.meta.url)); w.on('message', resolve); w.on('error', reject); }); return [first, second, third]; };"),
            ("a/worker.js", "var wt = require('node:worker_threads'); var child = new wt.Worker(new URL('./nested.js', import.meta.url)); child.on('message', function (value) { wt.parentPort.postMessage(value); wt.parentPort.close(); }); child.on('error', function (error) { throw error; });"),
            ("a/nested.js", "var parentPort = require('node:worker_threads').parentPort; var direct, created; try { require('affinity-missing'); } catch (error) { direct = error.code; } var makeRequire = require('node:module').createRequire(__filename); try { makeRequire('affinity-missing'); } catch (error) { created = error.code; } parentPort.postMessage([require('./dep.js').value, require('affinity-shared').value, direct, created]); parentPort.close();"),
            ("a/dep.js", "exports.value = 'A';"),
            ("a/node_modules/affinity-shared/package.json", r#"{"name":"affinity-shared","main":"index.js"}"#),
            ("a/node_modules/affinity-shared/index.js", "exports.value = 'A-bare';"),
            ("b/worker.js", "var parentPort = require('node:worker_threads').parentPort; parentPort.postMessage([require('./dep.js').value, require('affinity-shared').value]); parentPort.close();"),
            ("b/dep.js", "exports.value = 'B';"),
            ("b/node_modules/affinity-shared/package.json", r#"{"name":"affinity-shared","main":"index.js"}"#),
            ("b/node_modules/affinity-shared/index.js", "exports.value = 'B-bare';"),
        ],
    );
    let modules = temp_registry("bundled_worker_and_nested_worker_affinity_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(
        run_js(&bundle),
        r#"[["A","A-bare","MODULE_NOT_FOUND","MODULE_NOT_FOUND"],["B","B-bare"],["A","A-bare","MODULE_NOT_FOUND","MODULE_NOT_FOUND"]]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(modules);
}

#[test]
fn file_worker_and_main_keep_root_dirname_for_entry_and_dependencies() {
    let dir = worker_fixture(
        "worker_root_dirname",
        &[
            ("entry.js", "var wt = require('node:worker_threads'); wt.parentPort.postMessage([__dirname, require('./child.js')]); wt.parentPort.close();"),
            ("child.js", "module.exports = __dirname;"),
            ("nested/entry.js", "var wt = require('node:worker_threads'); wt.parentPort.postMessage([__dirname, require('./child.js')]); wt.parentPort.close();"),
            ("nested/child.js", "module.exports = __dirname;"),
        ],
    );
    let entry = serde_json::to_string(dir.join("entry.js").to_str().unwrap()).unwrap();
    let nested = serde_json::to_string(dir.join("nested/entry.js").to_str().unwrap()).unwrap();
    let result = run_worker_fixture(
        &dir,
        &format!(
            "var a = await runWorker({entry}), b = await runWorker({nested}), c = await runWorker('entry.js'); \
             return [a.messages, b.messages, c.syncCode];"
        ),
    );
    let root = dir.to_string_lossy().into_owned();
    let nested_dir = dir.join("nested").to_string_lossy().into_owned();
    assert_eq!(
        result,
        serde_json::json!([[[root, root]], [[nested_dir, nested_dir]], "ERR_WORKER_PATH"]).to_string()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn file_worker_resolves_parent_components_and_rejects_bare_relative_paths() {
    let dir = worker_fixture(
        "worker_parent_components",
        &[
            ("a/entry.js", "var wt = require('node:worker_threads'); wt.parentPort.postMessage(require('../child.js')); wt.parentPort.close();"),
            ("child.js", "module.exports = __filename;"),
        ],
    );
    let entry = serde_json::to_string(dir.join("a/entry.js").to_str().unwrap()).unwrap();
    let unnormalised = serde_json::to_string(dir.join("a/../a/entry.js").to_str().unwrap()).unwrap();
    let result = run_worker_fixture(
        &dir,
        &format!(
            "var a = await runWorker({entry}), b = await runWorker({unnormalised}), \
             c = await runWorker('entry.js'), d = await runWorker('nested/entry.js'), \
             e = await runWorker('../entry.js'); return [a, b, c, d, e];"
        ),
    );
    let child = dir.join("child.js").to_string_lossy().into_owned();
    assert_eq!(
        result,
        serde_json::json!([
            { "messages": [child], "errors": [] },
            { "messages": [child], "errors": [] },
            { "syncCode": "ERR_WORKER_PATH" },
            { "syncCode": "ERR_WORKER_PATH" },
            { "messages": [], "errors": ["MODULE_NOT_FOUND"] }
        ])
        .to_string()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn file_loaders_retry_failed_modules_and_bind_commonjs_this() {
    let dir = worker_fixture(
        "worker_cache_lifecycle",
        &[
            ("entry.js", "var wt = require('node:worker_threads');\n\
                var caught = false; try { require('./child.js'); } catch (e) { caught = e && e.m === 1; }\n\
                var second = require('./child.js'), third = require('./child.js');\n\
                var jf = false; try { require('./data.json'); } catch (e) { jf = e instanceof SyntaxError; }\n\
                wt.parentPort.postMessage('fix');\n\
                wt.parentPort.once('message', function () {\n\
                    wt.parentPort.postMessage([caught, second.cycle, second.published, second === third, globalThis.attempts, jf, require('./data.json').value]);\n\
                    wt.parentPort.close();\n\
                });"),
            ("child.js", "'use strict'; globalThis.attempts = (globalThis.attempts || 0) + 1; exports.started = true; this.published = 42; exports.cycle = require('./cycle.js'); if (globalThis.attempts === 1) throw { m: 1 };"),
            ("cycle.js", "module.exports = require('./child.js').started;"),
            ("data.json", "{broken"),
        ],
    );
    let entry = serde_json::to_string(dir.join("entry.js").to_str().unwrap()).unwrap();
    let data = serde_json::to_string(dir.join("data.json").to_str().unwrap()).unwrap();
    // The worker's first JSON read fails; the parent repairs data.json at the 'fix' handshake, then releases the worker.
    let result = run_worker_fixture(
        &dir,
        &format!(
            "var out = await runWorker({entry}, function (worker) {{ \
                require('node:fs').writeFileSync({data}, '{{\"value\":13}}'); worker.postMessage('go'); \
             }}); return [out.messages[0], out.errors];"
        ),
    );
    assert_eq!(
        result,
        serde_json::json!([[true, true, 42, true, 2, true, 13], []]).to_string()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn file_loaders_accept_hashbangs_without_stripping_json_or_interior_text() {
    let dir = worker_fixture(
        "worker_hashbang_lines",
        &[
            ("entry.js", "#!/usr/bin/env node\n\
                var wt = require('node:worker_threads'); var inv = false; try { require('./invalid.json'); } catch (e) { inv = e instanceof SyntaxError; } \
                wt.parentPort.postMessage([require('./lf.js'), require('./crlf.cjs'), require('./ls.js'), require('./ps.js'), Object.keys(require('./empty.js')).length, require('./text.js'), inv]); wt.parentPort.close();"),
            ("lf.js", "#!/usr/bin/env node\nmodule.exports = 1;"),
            ("crlf.cjs", "#!/usr/bin/env node\r\nmodule.exports = 2;"),
            ("ls.js", "#!/usr/bin/env node\u{2028}module.exports = 3;"),
            ("ps.js", "#!/usr/bin/env node\u{2029}module.exports = 4;"),
            ("empty.js", "#!/usr/bin/env node"),
            ("text.js", "module.exports = \"#!kept\";"),
            ("invalid.json", "#!/usr/bin/env node\n{\"value\":1}"),
        ],
    );
    let entry = serde_json::to_string(dir.join("entry.js").to_str().unwrap()).unwrap();
    let result = run_worker_fixture(&dir, &format!("var out = await runWorker({entry}); return [out.messages[0], out.errors];"));
    assert_eq!(
        result,
        serde_json::json!([[1, 2, 3, 4, 0, "#!kept", true], []]).to_string()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn file_worker_package_main_rethrows_initialization_and_syntax_errors() {
    const MAIN_JS: &str = "var counts = globalThis.__counts || (globalThis.__counts = {}); if ((counts[__filename] = (counts[__filename] || 0) + 1) === 1) throw globalThis.__fail; module.exports = 42;";
    let dir = worker_fixture(
        "worker_package_main_errors",
        &[
            (
                "entry.js",
                "var wt = require('node:worker_threads'); var fail = globalThis.__fail = {m: 1}; function retry(request) { var caught = false; try { require(request); } catch (error) { caught = error === fail; } return [caught, require(request)]; } var syntaxFailed = false; try { require('./syntax-package'); } catch (error) { syntaxFailed = error instanceof SyntaxError; } wt.parentPort.postMessage([retry('./local-package'), retry('bare-package'), syntaxFailed]); wt.parentPort.close();",
            ),
            ("local-package/package.json", r#"{"main":"main.js"}"#),
            ("local-package/main.js", MAIN_JS),
            ("node_modules/bare-package/package.json", r#"{"main":"main.js"}"#),
            ("node_modules/bare-package/main.js", MAIN_JS),
            ("syntax-package/package.json", r#"{"main":"broken.js"}"#),
            ("syntax-package/broken.js", "module.exports = ("),
        ],
    );
    let entry = serde_json::to_string(dir.join("entry.js").to_str().unwrap()).unwrap();
    let result = run_worker_fixture(&dir, &format!("var r = await runWorker({entry}); return r.messages;"));
    assert_eq!(result, serde_json::json!([[[true, 42], [true, 42], true]]).to_string());
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn file_loaders_resolve_dotted_files_and_directories_in_file_first_order() {
    let dir = worker_fixture(
        "worker_dotted_resolution",
        &[
            (
                "entry.js",
                "var wt = require('node:worker_threads'); var out = []; ['./helper.test', './data.v1', './dir.v1', './cjs.v1', './json.v1', './pkg.v1', './collision', './exact.v1'].forEach(function (request) { try { out.push(require(request)); } catch (error) { out.push(error.code); } }); wt.parentPort.postMessage(out); wt.parentPort.close();",
            ),
            ("helper.test.js", "module.exports = 11;"),
            ("data.v1.json", "12"),
            ("dir.v1/index.js", "module.exports = 13;"),
            ("cjs.v1/index.cjs", "module.exports = 14;"),
            ("json.v1/index.json", "15"),
            ("pkg.v1/package.json", r#"{"main":"main"}"#),
            ("pkg.v1/main.js", "module.exports = 16;"),
            ("collision.js", "module.exports = 17;"),
            ("collision/package.json", r#"{"main":"wrong.js"}"#),
            ("collision/wrong.js", "module.exports = 99;"),
            ("exact.v1", "module.exports = 18;"),
        ],
    );
    let entry = serde_json::to_string(dir.join("entry.js").to_str().unwrap()).unwrap();
    let result = run_worker_fixture(&dir, &format!("var r = await runWorker({entry}); return r.messages;"));
    assert_eq!(
        result,
        serde_json::json!([[11, 12, 13, "MODULE_NOT_FOUND", 15, 16, 17, 18]]).to_string()
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn file_worker_keeps_directives_and_fallback_scope_in_entry_wrapper() {
    let dir = temp_registry("worker_entry_directives");
    let modules = temp_registry("worker_entry_directives_modules");
    let body = r#"var object = {}; Object.defineProperty(object, 'fixed', { value: 1, writable: false });
var rejectedWrite = false; try { object.fixed = 2; } catch (error) { rejectedWrite = error instanceof TypeError; }
var values = [rejectedWrite, (function() { return this === undefined; })(), this === exports, require('./dep.js')];
var port = require('node:worker_threads').parentPort; port.postMessage(values); port.close();
"#;
    fs::write(dir.join("strict.js"), format!("/* leading comment */ 'other directive'; 'use strict';\n{body}")).unwrap();
    fs::write(dir.join("sloppy.js"), format!("\n{body}")).unwrap();
    fs::write(dir.join("escaped.js"), format!("'use\\x20strict';\n{body}")).unwrap();
    fs::write(dir.join("dep.js"), "'use strict'; module.exports = (function() { return this === undefined; })();\n").unwrap();
    let paths = ["strict.js", "sloppy.js", "escaped.js"]
        .map(|f| format!("{:?}", dir.join(f).display().to_string()))
        .join(", ");
    fs::write(
        dir.join("index.js"),
        format!(
            "var Worker = require('node:worker_threads').Worker; \
             function runWorker(path) {{ return new Promise(function (resolve, reject) {{ var worker = new Worker(path), message; \
             worker.on('message', function (value) {{ message = value; }}); worker.on('error', reject); \
             worker.on('exit', function () {{ resolve(message); }}); }}); }} \
             module.exports = async function () {{ var out = []; for (var f of [{paths}]) out.push(await runWorker(f)); return out; }};"
        ),
    )
    .unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(
        run_js(&bundle),
        "[[true,true,true,true],[false,false,true,true],[false,false,true,true]]"
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(modules);
}

#[test]
fn worker_thread_id_becomes_minus_one_after_routing_cleanup() {
    let dir = temp_registry("worker_terminal_thread_id");
    let modules = temp_registry("worker_terminal_thread_id_modules");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker; module.exports = async function () { var normal = new Worker("require('node:worker_threads').parentPort.close();", { eval: true }); var normalId = normal.threadId, normalObserved; await new Promise(function(resolve, reject) { normal.on('error', reject); normal.on('exit', function() { normalObserved = normal.threadId; resolve(); }); }); var stopped = new Worker("require('node:worker_threads').parentPort.on('message', function() {});", { eval: true }); await new Promise(function(resolve, reject) { stopped.on('error', reject); stopped.on('online', resolve); }); var stoppedId = stopped.threadId; await stopped.terminate(); var stoppedAfterRepeat = await stopped.terminate(); var native = new Worker("void 0;", { eval: true }); var nativeId = native.threadId, nativeObserved; var nativeExit = new Promise(function(resolve, reject) { native.on('error', reject); native.on('exit', function() { nativeObserved = native.threadId; resolve(); }); }); var termination = native.terminate(); await Promise.all([nativeExit, termination]); var nativeRepeated = await native.terminate(); return [normalId > 0, normalObserved, stoppedId > 0, stopped.threadId, stoppedAfterRepeat === undefined, nativeId > 0, nativeObserved, native.threadId, nativeRepeated === undefined]; };"#,
    )
    .unwrap();
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &dir, "index.js").unwrap();
    // Node 22: a repeated terminate() resolves undefined (the registry expected the exit code 1 and 0).
    assert_eq!(
        run_js(&bundle),
        "[true,-1,true,-1,true,true,-1,-1,true]"
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(modules);
}


#[test]
fn nested_worker_runs_to_exit_and_reports_child_exit_code() {
    // Node 22.23.1: [0,0] (the child's exit code, then the parent's exit code)
    let dir = temp_registry("nested_worker_runs_to_exit");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
        module.exports = async function() {
            var source = "var wt = require('node:worker_threads'), child = new wt.Worker(\"require('node:worker_threads').parentPort.close();\", { eval: true }); child.on('exit', function(code) { wt.parentPort.postMessage(code); wt.parentPort.close(); });";
            var worker = new Worker(source, { eval: true });
            var message = await new Promise(function(resolve, reject) {
                worker.on('message', resolve);
                worker.on('error', reject);
            });
            var exit = await new Promise(function(resolve) { worker.on('exit', resolve); });
            return [message, exit];
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_runs_to_exit_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), "[0,0]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn nested_workers_keep_parent_alive_unref_releases_it_and_failed_ctor_is_caught() {
    // Node 22.23.1: [0,[0,[0,"replacement:0"]],0] (a failed constructor is caught, a ref'd child keeps its parent
    // alive, an unref'd child does not)
    let dir = temp_registry("nested_worker_lifetime");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
        module.exports = async function() {
            function run(source, captureMessage) {
                var worker = new Worker(source, { eval: true });
                return new Promise(function(resolve) {
                    var done = false, messages = [], timer = setTimeout(function() {
                        if (done) return;
                        done = true;
                        resolve(-1);
                        worker.terminate();
                    }, 5000);
                    worker.on('message', function(value) { messages.push(value); });
                    worker.on('exit', function(code) {
                        if (done) return;
                        done = true;
                        clearTimeout(timer);
                        resolve(captureMessage ? [code, messages] : code);
                    });
                });
            }
            var failedConstructor = "var wt = require('node:worker_threads'); try { new wt.Worker('void 0', { eval: true, workerData: function() {} }); } catch (error) {}";
            var nestedChild = "var wt = require('node:worker_threads'), child = new wt.Worker(\"require('node:worker_threads').parentPort.close();\", { eval: true }); child.on('exit', function(code) { wt.parentPort.postMessage(code); var replacement = new wt.Worker(\"require('node:worker_threads').parentPort.close();\", { eval: true }); replacement.on('exit', function(replacementCode) { wt.parentPort.postMessage('replacement:' + replacementCode); wt.parentPort.close(); }); }); child.unref(); child.ref();";
            var unrefChild = "var wt = require('node:worker_threads'), child = new wt.Worker(\"setTimeout(function() { require('node:worker_threads').parentPort.close(); }, 50);\", { eval: true }); child.unref();";
            return [await run(failedConstructor), await run(nestedChild, true), await run(unrefChild)];
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_lifetime_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), "[0,[0,[0,\"replacement:0\"]],0]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn nested_worker_child_inherits_env_copy_and_streams_stdout_to_parent_worker() {
    // Node 22.23.1: ["fallback-parent-env","child-stdio",0]; the child's stdout reaches the top worker's stdout capture
    let dir = temp_registry("nested_worker_inherited_state");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
        module.exports = async function() {
            var source = "var wt = require('node:worker_threads'), child = new wt.Worker(\"process.stdout.write('child-stdio'); require('node:worker_threads').parentPort.postMessage(process.env.THAW_NESTED); require('node:worker_threads').parentPort.close();\", { eval: true }); child.on('message', function(value) { wt.parentPort.postMessage(value); }); child.on('exit', function() { wt.parentPort.close(); });";
            var worker = new Worker(source, { eval: true, env: { THAW_NESTED: 'fallback-parent-env' }, stdout: true });
            var output = '';
            worker.stdout.setEncoding('utf8');
            worker.stdout.on('data', function(value) { output += value; });
            var message = new Promise(function(resolve, reject) { worker.on('message', resolve); worker.on('error', reject); });
            var outputDone = new Promise(function(resolve) { worker.stdout.on('end', resolve); });
            var exit = new Promise(function(resolve, reject) { worker.on('exit', resolve); worker.on('error', reject); });
            var values = await Promise.all([message, outputDone, exit]);
            return [values[0], output, values[2]];
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_inherited_state_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), "[\"fallback-parent-env\",\"child-stdio\",0]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn nested_worker_thread_ids_are_one_tree_wide_sequence() {
    // Node 22.23.1 (3 runs identical): the top worker is 1, its children 2 and 3 (one counter for the tree, in creation
    // order); a child is not the main thread, has its workerData, an empty threadName and a parentPort; a worker's
    // threadId reads -1 once it has exited.
    let dir = temp_registry("nested_worker_thread_ids");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
        module.exports = async function() {
            var kid = "var wt = require('node:worker_threads'); wt.parentPort.postMessage({ me: wt.threadId, main: wt.isMainThread, data: wt.workerData, name: wt.threadName, parent: wt.parentPort !== null }); wt.parentPort.close();";
            var mid = "var wt = require('node:worker_threads'); var results = [], ids = [], kids = []; var pending = 2; var kidCode = `" + kid + "`; ['k1', 'k2'].forEach(function (name, index) { var child = new wt.Worker(kidCode, { eval: true, workerData: name }); kids.push(child); ids.push(child.threadId); child.on('message', function (m) { results[index] = m; }); child.on('exit', function () { if (--pending === 0) { wt.parentPort.postMessage({ me: wt.threadId, ids: ids, results: results, after: kids.map(function (k) { return k.threadId; }) }); wt.parentPort.close(); } }); });";
            var top = new Worker(mid, { eval: true, workerData: 'mid' });
            var topId = top.threadId;
            var message = await new Promise(function(resolve, reject) { top.on('message', resolve); top.on('error', reject); });
            await new Promise(function(resolve) { top.on('exit', resolve); });
            return [topId, message, top.threadId];
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_thread_ids_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(
        run_js(&bundle),
        r#"[1,{"me":1,"ids":[2,3],"results":[{"me":2,"main":false,"data":"k1","name":"","parent":true},{"me":3,"main":false,"data":"k2","name":"","parent":true}],"after":[-1,-1]},-1]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn terminating_a_worker_terminates_its_children() {
    // Node 22.23.1 (3 runs identical): terminate() resolves 1, and the grandchild's SharedArrayBuffer heartbeat is
    // frozen once it has resolved (it was running before: frozen > 0)
    let dir = temp_registry("nested_worker_terminate_cascade");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
        var kid = "var wt = require('node:worker_threads'), hb = new Int32Array(wt.workerData); setInterval(function () { Atomics.add(hb, 0, 1); }, 10);";
        var mid = "var wt = require('node:worker_threads'); var c = new wt.Worker(" + JSON.stringify(kid) + ", { eval: true, workerData: wt.workerData }); c.on('online', function () { wt.parentPort.postMessage('child-ready'); });";
        module.exports = async function () {
            var sab = new SharedArrayBuffer(4), hb = new Int32Array(sab);
            var top = new Worker(mid, { eval: true, workerData: sab });
            await new Promise(function (resolve, reject) {
                top.on('message', function (m) { if (m === 'child-ready') resolve(); });
                top.on('error', reject);
            });
            await new Promise(function (r) { setTimeout(r, 200); });
            var code = await top.terminate();
            var frozen = Atomics.load(hb, 0);
            await new Promise(function (r) { setTimeout(r, 300); });
            return [code, frozen > 0, Atomics.load(hb, 0) === frozen];
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_terminate_cascade_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), "[1,true,true]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn terminated_worker_does_not_deliver_its_children_exit_events() {
    // Node 22.23.1 (3 runs identical): the child is stopped with its parent, and its 'exit' never reaches the parent's JS
    let dir = temp_registry("nested_worker_terminate_child_exit");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
        var kid = "setInterval(function () {}, 1000);";
        var mid = "var wt = require('node:worker_threads'); var c = new wt.Worker(" + JSON.stringify(kid) + ", { eval: true }); c.on('exit', function (code) { wt.parentPort.postMessage(['child exit', code]); }); wt.parentPort.postMessage('ready');";
        module.exports = async function () {
            var messages = [];
            var top = new Worker(mid, { eval: true });
            await new Promise(function (resolve) {
                top.on('message', function (m) { messages.push(m); if (m === 'ready') setTimeout(resolve, 200); });
            });
            var code = await top.terminate();
            await new Promise(function (r) { setTimeout(r, 300); });
            return [code, messages];
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_terminate_child_exit_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), r#"[1,["ready"]]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn nested_worker_exit_code_and_error_reach_its_parent() {
    // Node 22.23.1 (3 runs identical; the event order between the two children varies, so the log is sorted):
    // process.exit(7) in a child is its 'exit' 7; a throw is its parent's 'error' then 'exit' 1; the top worker exits 0
    let dir = temp_registry("nested_worker_exit_and_error");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
        var mid = "var wt = require('node:worker_threads'); var c = new wt.Worker('process.exit(7);', { eval: true }); var d = new wt.Worker('throw new Error(\"boom\");', { eval: true }); c.on('exit', function (code) { wt.parentPort.postMessage(['c exit', code]); }); d.on('error', function (e) { wt.parentPort.postMessage(['d error', e.message]); }); d.on('exit', function (code) { wt.parentPort.postMessage(['d exit', code]); });";
        module.exports = async function () {
            var log = [];
            var top = new Worker(mid, { eval: true });
            top.on('message', function (m) { log.push(m); });
            var code = await new Promise(function (r) { top.on('exit', r); });
            log.sort(function (a, b) { var x = JSON.stringify(a), y = JSON.stringify(b); return x < y ? -1 : x > y ? 1 : 0; });
            return [code, log];
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_exit_and_error_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(
        run_js(&bundle),
        r#"[0,[["c exit",7],["d error","boom"],["d exit",1]]]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn nested_worker_share_env_writes_reach_the_main_environment() {
    // Node 22.23.1 (3 runs identical): a SHARE_ENV child of a SHARE_ENV worker writes the main thread's process.env
    let dir = temp_registry("nested_worker_share_env");
    fs::write(
        dir.join("index.js"),
        r#"var wt = require('node:worker_threads');
        var kid = "process.env.THAW_N2_NESTED = 'gc';";
        var mid = "var wt = require('node:worker_threads'); var c = new wt.Worker(" + JSON.stringify(kid) + ", { eval: true, env: wt.SHARE_ENV });";
        module.exports = async function () {
            var top = new wt.Worker(mid, { eval: true, env: wt.SHARE_ENV });
            await new Promise(function (r) { top.on('exit', r); });
            var seen = process.env.THAW_N2_NESTED;
            delete process.env.THAW_N2_NESTED;
            return seen;
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_share_env_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), "gc");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn terminating_a_worker_stops_a_child_blocked_in_atomics_wait() {
    // Node 22.23.1 (3 runs identical): the grandchild's Atomics.wait is ended by the terminate; terminate() resolves 1
    let dir = temp_registry("nested_worker_terminate_atomics_wait");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
        var kid = "var wt = require('node:worker_threads'); var hb = new Int32Array(wt.workerData); Atomics.wait(hb, 0, 0);";
        var mid = "var wt = require('node:worker_threads'); var c = new wt.Worker(" + JSON.stringify(kid) + ", { eval: true, workerData: wt.workerData }); wt.parentPort.postMessage('ready');";
        module.exports = async function () {
            var top = new Worker(mid, { eval: true, workerData: new SharedArrayBuffer(4) });
            await new Promise(function (resolve) { top.on('message', function (m) { if (m === 'ready') setTimeout(resolve, 300); }); });
            var code = await top.terminate();
            return [code];
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_terminate_atomics_wait_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), "[1]");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn nested_worker_port_routes_from_grandchild_through_middle_worker() {
    // Node 22.23.1 (3 runs identical, nw4-probes/p6.js and p13.js): a port moved main -> w1 -> its child (thread 3) answers
    // on the main-thread end, and the main thread's ping reaches the child back through w1. w2 is created second, so the
    // child is the third thread of the tree.
    let dir = temp_registry("nested_worker_port_route");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
        var kid = "var wt = require('node:worker_threads'); wt.parentPort.on('message', function (m) { if (m.port) { m.port.on('message', function (v) { if (v === 'ping') { m.port.postMessage({ echo: 'pong', tid: wt.threadId }); m.port.close(); wt.parentPort.close(); } }); m.port.postMessage({ hello: 'from kid via main-hub', tid: wt.threadId }); } });";
        var mid = "var wt = require('node:worker_threads'); var c = new wt.Worker(" + JSON.stringify(kid) + ", { eval: true }); wt.parentPort.on('message', function (m) { if (m.port) c.postMessage({ port: m.port }, [m.port]); }); c.on('exit', function () { wt.parentPort.close(); });";
        module.exports = async function () {
            var w1 = new Worker(mid, { eval: true });
            var w2 = new Worker("require('node:worker_threads').parentPort.on('message', function () {});", { eval: true });
            var channel = new MessageChannel();
            var messages = [];
            var done = new Promise(function (resolve) {
                channel.port1.on('message', function (m) {
                    messages.push(m);
                    // main -> kid -> main: the kid's hello comes first, then its echo to our ping
                    if (messages.length === 1) channel.port1.postMessage('ping');
                    else resolve(messages);
                });
            });
            w1.postMessage({ port: channel.port2 }, [channel.port2]);
            var value = await Promise.race([done, new Promise(function (r) { setTimeout(function () { r('timeout'); }, 3000); })]);
            channel.port1.close();
            await Promise.all([w1.terminate(), w2.terminate()]);
            return value;
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_port_route_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(
        run_js(&bundle),
        r#"[{"hello":"from kid via main-hub","tid":3},{"echo":"pong","tid":3}]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn nested_worker_cousin_post_message_reaches_the_sibling_subtree() {
    // Node 22.23.1 (3 runs identical, nw4-probes/p6.js): thread 3 posts to thread 2, its cousin (neither its parent nor
    // its child). The receiver sees {x:1} from 3 before the sender's ack reaches the sender's parent.
    let dir = temp_registry("nested_worker_cousin_message");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
        var kid = "var wt = require('node:worker_threads'); wt.parentPort.on('message', function (m) { if (m.cmd === 'to') { wt.postMessageToThread(m.target, { x: 1 }, [], 1000).then(function () { wt.parentPort.postMessage('cousin ok'); wt.parentPort.close(); }, function (e) { wt.parentPort.postMessage('cousin ' + e.code); wt.parentPort.close(); }); } });";
        var mid = "var wt = require('node:worker_threads'); var c = new wt.Worker(" + JSON.stringify(kid) + ", { eval: true }); wt.parentPort.on('message', function (m) { if (m.cmd === 'to') c.postMessage(m); }); c.on('message', function (m) { wt.parentPort.postMessage(m); }); c.on('exit', function () { wt.parentPort.close(); });";
        module.exports = async function () {
            var w1 = new Worker(mid, { eval: true });
            var receiverSource = "var wt = require('node:worker_threads'); process.on('workerMessage', function (v, src) { wt.parentPort.postMessage({ got: v, src: src }); }); wt.parentPort.postMessage('ready'); wt.parentPort.on('message', function (m) { if (m === 'quit') wt.parentPort.close(); });";
            var w2 = new Worker(receiverSource, { eval: true });
            var resolveReady;
            var ready = new Promise(function (r) { resolveReady = r; });
            var seen = [];
            var resolveBoth;
            var both = new Promise(function (r) { resolveBoth = r; });
            var collect = function (m) { seen.push(m); if (seen.length === 2) resolveBoth(); };
            w1.on('message', collect);
            w2.on('message', function (m) { if (m === 'ready') resolveReady(); else collect(m); });
            await ready;
            w1.postMessage({ cmd: 'to', target: w2.threadId });
            var result = await Promise.race([both.then(function () { return seen; }), new Promise(function (r) { setTimeout(function () { r(['timeout']); }, 3000); })]);
            await Promise.all([w1.terminate(), w2.terminate()]);
            // the receiver's message and the sender's ack race across two threads; compare in a fixed order
            return result.sort(function (x, y) { return (typeof x === 'string') - (typeof y === 'string'); });
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_cousin_message_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), r#"[{"got":{"x":1},"src":3},"cousin ok"]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn nested_worker_post_message_to_main_thread_reaches_its_workermessage() {
    // Node 22.23.1 (3 runs identical, nw4-probes/p7.js): a grandchild's postMessageToThread(0) fires the main thread's
    // process 'workerMessage' with the grandchild's thread id (2) as the source
    let dir = temp_registry("nested_worker_to_main_message");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
        var grandchild = "var wt = require('node:worker_threads'); wt.postMessageToThread(0, { hi: 'grandchild' }).then(function () {}, function () {}); wt.parentPort.close();";
        var mid = "var wt = require('node:worker_threads'); var c = new wt.Worker(" + JSON.stringify(grandchild) + ", { eval: true }); c.on('exit', function () { wt.parentPort.close(); });";
        module.exports = async function () {
            var got = new Promise(function (resolve) { process.on('workerMessage', function (value, source) { resolve([value, source]); }); });
            var top = new Worker(mid, { eval: true });
            var result = await Promise.race([got, new Promise(function (r) { setTimeout(function () { r('no workerMessage'); }, 3000); })]);
            await top.terminate();
            return result;
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_to_main_message_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(run_js(&bundle), r#"[{"hi":"grandchild"},2]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}

#[test]
fn nested_worker_port_closes_for_its_peer_when_its_owner_stops() {
    // Node 22.23.1 (3 runs identical, nw4-probes/p12.js). A: a started port owned by a worker that exits by itself: exit,
    // then the peer's close. B: the owner is a child of a parent that keeps running: the peer closes, then the parent
    // exits when told to. C: the owner is a child of a parent that is terminated: the peer closes before the parent's
    // exit, and terminate() resolves 1.
    let dir = temp_registry("nested_worker_port_close");
    fs::write(
        dir.join("index.js"),
        r#"var Worker = require('node:worker_threads').Worker;
        var owner = "var wt = require('node:worker_threads'); wt.parentPort.on('message', function (m) { if (m.port) { m.port.on('message', function () {}); m.port.unref(); wt.parentPort.close(); } });";
        var ownerStays = "var wt = require('node:worker_threads'); wt.parentPort.on('message', function (m) { if (m.port) { m.port.on('message', function () {}); m.port.unref(); } });";
        var midB = "var wt = require('node:worker_threads'); var c = new wt.Worker(" + JSON.stringify(owner) + ", { eval: true }); wt.parentPort.on('message', function (m) { if (m.port) c.postMessage({ port: m.port }, [m.port]); if (m === 'stop') wt.parentPort.close(); });";
        var midC = "var wt = require('node:worker_threads'); var c = new wt.Worker(" + JSON.stringify(ownerStays) + ", { eval: true }); wt.parentPort.on('message', function (m) { if (m.port) { c.postMessage({ port: m.port }, [m.port]); wt.parentPort.postMessage('kid-holds'); } });";
        module.exports = async function () {
            var watchdog = function () { return new Promise(function (r) { setTimeout(function () { r('timeout'); }, 3000); }); };
            var ports = [], workers = [];
            var closed = function (port, log, label) { return new Promise(function (r) { port.on('close', function () { log.push(label + ' close'); r(); }); }); };
            var logA = [], a = new MessageChannel();
            ports.push(a.port1); a.port1.on('message', function () {});
            var wa = new Worker(owner, { eval: true }); workers.push(wa);
            wa.on('exit', function (code) { logA.push('A exit ' + code); });
            var aClosed = closed(a.port1, logA, 'A');
            wa.postMessage({ port: a.port2 }, [a.port2]);
            await Promise.race([aClosed, watchdog()]);
            var logB = [], b = new MessageChannel();
            ports.push(b.port1); b.port1.on('message', function () {});
            var wb = new Worker(midB, { eval: true }); workers.push(wb);
            var bExit = new Promise(function (r) { wb.on('exit', function (code) { logB.push('B mid exit ' + code); r(); }); });
            var bClosed = closed(b.port1, logB, 'B');
            wb.postMessage({ port: b.port2 }, [b.port2]);
            await Promise.race([bClosed, watchdog()]);
            wb.postMessage('stop');
            await Promise.race([bExit, watchdog()]);
            var logC = [], c = new MessageChannel();
            ports.push(c.port1); c.port1.on('message', function () {});
            var wc = new Worker(midC, { eval: true }); workers.push(wc);
            var cExit = new Promise(function (r) { wc.on('exit', function (code) { logC.push('C exit ' + code); r(); }); });
            var cClosed = closed(c.port1, logC, 'C');
            var cCode;
            wc.on('message', function (m) { if (m === 'kid-holds') setTimeout(function () { wc.terminate().then(function (code) { cCode = code; }); }, 200); });
            wc.postMessage({ port: c.port2 }, [c.port2]);
            await Promise.race([Promise.all([cClosed, cExit]), watchdog()]);
            ports.forEach(function (port) { port.close(); });
            await Promise.all(workers.map(function (w) { return w.terminate(); }));
            return [logA, logB, logC, cCode];
        };"#,
    )
    .unwrap();
    let node_modules = temp_registry("nested_worker_port_close_node_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(
        run_js(&bundle),
        r#"[["A exit 0","A close"],["B close","B mid exit 0"],["C close","C exit 1"],1]"#
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(node_modules);
}


// node:zlib zstd (PR-1: constants, sync and callback one-shots). Expected bytes and error shapes were recorded from
// Node 22.23.1 (libzstd 1.5.7), the same library thaw-js links through zstd-safe.

#[cfg(feature = "zstd")]
const ZSTD_HELLO_SYNC: &str = "28b52ffd201c8500005068656c6c6f207a7374640100f14a11";
#[cfg(feature = "zstd")]
const ZSTD_HELLO_CALLBACK: &str = "28b52ffd00588500005068656c6c6f207a7374640100f14a11";

/// Runs `body` (a JS expression, or a `return` body for async functions) as an npm-style CommonJS package and returns
/// its JSON result.
fn zstd_run(name: &str, body: &str) -> String {
    let dir = temp_registry(name);
    fs::write(
        dir.join("index.js"),
        format!("var zlib = require('node:zlib'), s = Buffer.from('hello hello hello hello zstd'); module.exports = async function () {{ {body} }};"),
    )
    .unwrap();
    let empty_node_modules = temp_registry(&format!("{name}_node_modules"));
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let result = run_js(&bundle);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    result
}

#[test]
fn zlib_zstd_constants_match_node() {
    let result = zstd_run(
        "zstd_constants",
        "var c = zlib.constants; return [c.ZSTD_e_end, c.ZSTD_e_continue, c.ZSTD_c_compressionLevel, c.ZSTD_d_windowLogMax, c.ZSTD_error_prefix_unknown, c.ZSTD_error_noForwardProgress_inputEmpty, c.ZSTD_CLEVEL_DEFAULT, Object.keys(c).filter(function (k) { return k.indexOf('ZSTD') === 0; }).length];",
    );
    assert_eq!(result, "[2,0,100,100,10,82,3,63]");
}

#[cfg(not(feature = "zstd"))]
#[test]
fn zlib_zstd_functions_are_absent_without_the_native() {
    let result = zstd_run(
        "zstd_absent",
        "return [typeof zlib.zstdCompressSync, typeof zlib.zstdDecompressSync, typeof zlib.zstdCompress, typeof zlib.zstdDecompress];",
    );
    assert_eq!(result, r#"["undefined","undefined","undefined","undefined"]"#);
}

#[cfg(feature = "zstd")]
#[test]
fn zlib_zstd_sync_compress_matches_node_bytes_at_each_level() {
    let result = zstd_run(
        "zstd_sync_levels",
        "var hex = function (level) { return zlib.zstdCompressSync(s, { params: { [zlib.constants.ZSTD_c_compressionLevel]: level } }).toString('hex'); }; return [hex(1), hex(3), hex(19), zlib.zstdCompressSync(s).toString('hex')];",
    );
    assert_eq!(
        result,
        format!(r#"["{h}","{h}","{h}","{h}"]"#, h = ZSTD_HELLO_SYNC)
    );
}

#[cfg(feature = "zstd")]
#[test]
fn zlib_zstd_decodes_node_frames() {
    let result = zstd_run(
        "zstd_decode_node",
        &format!(
            "return [zlib.zstdDecompressSync(Buffer.from('{ZSTD_HELLO_SYNC}', 'hex')).toString(), zlib.zstdDecompressSync(Buffer.from('{ZSTD_HELLO_CALLBACK}', 'hex')).toString()];"
        ),
    );
    assert_eq!(
        result,
        r#"["hello hello hello hello zstd","hello hello hello hello zstd"]"#
    );
}

#[cfg(feature = "zstd")]
#[test]
fn zlib_zstd_empty_input_round_trips_and_decodes_to_nothing() {
    let result = zstd_run(
        "zstd_empty",
        "return [zlib.zstdCompressSync(Buffer.alloc(0)).toString('hex'), zlib.zstdDecompressSync(Buffer.from('28b52ffd2000010000', 'hex')).length, zlib.zstdDecompressSync(Buffer.alloc(0)).length];",
    );
    assert_eq!(result, r#"["28b52ffd2000010000",0,0]"#);
}

#[cfg(feature = "zstd")]
#[test]
fn zlib_zstd_one_mib_round_trips_with_node_sizes() {
    let result = zstd_run(
        "zstd_one_mib",
        "var big = Buffer.from('the quick brown fox '.repeat(52429).slice(0, 1048576)); return [1, 3, 19].map(function (level) { var c = zlib.zstdCompressSync(big, { params: { 100: level } }); return [c.length, zlib.zstdDecompressSync(c).equals(big)]; });",
    );
    assert_eq!(result, "[[125,true],[124,true],[117,true]]");
}

#[cfg(feature = "zstd")]
#[test]
fn zlib_zstd_corrupt_and_wrong_magic_errors_have_node_shape() {
    let result = zstd_run(
        "zstd_errors",
        "var checksummed = Buffer.from('28b52ffd241c8500005068656c6c6f207a7374640100f14a11de38b055', 'hex'); checksummed[9] ^= 0x55; var corrupt = (function () { try { zlib.zstdDecompressSync(checksummed); return null; } catch (e) { return [e.name, e.code, e.errno, e.message]; } })(); var magic = (function () { try { zlib.zstdDecompressSync(Buffer.from('not zstd data')); return null; } catch (e) { return [e.name, e.code, e.errno, e.message]; } })(); return [corrupt, magic];"
    );
    assert_eq!(
        result,
        r#"[["Error","ZSTD_error_corruption_detected",20,"Data corruption detected"],["Error","ZSTD_error_prefix_unknown",10,"Unknown frame descriptor"]]"#
    );
}

#[cfg(feature = "zstd")]
#[test]
fn zlib_zstd_level_is_clamped_like_node() {
    let result = zstd_run(
        "zstd_clamp",
        "return [999, -5, 22].map(function (level) { return zlib.zstdCompressSync(s, { params: { 100: level } }).toString('hex'); });",
    );
    assert_eq!(
        result,
        format!(r#"["{h}","{h}","{h}"]"#, h = ZSTD_HELLO_SYNC)
    );
}

#[cfg(feature = "zstd")]
#[test]
fn zlib_zstd_truncated_concatenated_and_trailing_input_follow_node() {
    let result = zstd_run(
        "zstd_stream_shapes",
        "var frame = zlib.zstdCompressSync(s); return [zlib.zstdDecompressSync(frame.subarray(0, 10)).length, zlib.zstdDecompressSync(Buffer.concat([frame, frame])).toString(), zlib.zstdDecompressSync(Buffer.concat([frame, Buffer.from([1, 2, 3])])).toString()];",
    );
    assert_eq!(
        result,
        r#"[0,"hello hello hello hello zstdhello hello hello hello zstd","hello hello hello hello zstd"]"#
    );
}

#[cfg(feature = "zstd")]
#[test]
fn zlib_zstd_callback_api_uses_the_streaming_header_and_reports_errors() {
    let result = zstd_run(
        "zstd_callback",
        "var compress = function (value, options) { return new Promise(function (resolve, reject) { zlib.zstdCompress(value, options, function (error, output) { if (error) reject(error); else resolve(output.toString('hex')); }); }); }; var plain = await compress(s); var level19 = await compress(s, { params: { 100: 19 } }); var empty = await compress(Buffer.alloc(0)); var failure = await new Promise(function (resolve) { zlib.zstdDecompress(Buffer.from('not zstd data'), function (error) { resolve([error.code, error.errno, error.message]); }); }); return [plain, level19, empty, failure];",
    );
    assert_eq!(
        result,
        format!(
            r#"["{ZSTD_HELLO_CALLBACK}","28b52ffd00688500005068656c6c6f207a7374640100f14a11","28b52ffd0058010000",["ZSTD_error_prefix_unknown",10,"Unknown frame descriptor"]]"#
        )
    );
}

#[cfg(feature = "zstd")]
#[test]
fn zlib_zstd_option_errors_have_node_classes_and_messages() {
    let result = zstd_run(
        "zstd_option_errors",
        "var shape = function (fn) { try { fn(); return null; } catch (e) { return [e.name, e.code, e.message]; } }; return [shape(function () { zlib.zstdCompressSync(s, { params: { 999: 1 } }); }), shape(function () { zlib.zstdCompressSync(s, { params: { 100: 'x' } }); }), shape(function () { zlib.zstdCompressSync(s, { params: { 101: 5 } }); }), shape(function () { zlib.zstdDecompressSync(zlib.zstdCompressSync(s), { params: { 101: 20 } }); }), shape(function () { zlib.zstdDecompressSync(zlib.zstdCompressSync(s), { maxOutputLength: 10 }); })];",
    );
    assert_eq!(
        result,
        r#"[["RangeError","ERR_ZSTD_INVALID_PARAM","999 is not a valid zstd parameter"],["TypeError","ERR_INVALID_ARG_TYPE","The \"options.params[key]\" property must be of type number. Received type string ('x')"],["Error","ERR_ZLIB_INITIALIZATION_FAILED","Setting parameter failed"],["RangeError","ERR_ZSTD_INVALID_PARAM","101 is not a valid zstd parameter"],["RangeError","ERR_BUFFER_TOO_LARGE","Cannot create a Buffer larger than 10 bytes"]]"#
    );
}
