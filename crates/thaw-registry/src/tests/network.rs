#[test]
fn util_types_identifies_standard_and_typed_objects() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_util_types");
    fs::write(dir.join("index.js"), "var types = require('node:util/types'); module.exports = async function () { return [types.isDate(new Date()), types.isRegExp(/x/), types.isMap(new Map()), types.isSet(new Set()), types.isWeakMap(new WeakMap()), types.isPromise(Promise.resolve()), types.isArrayBuffer(new ArrayBuffer(2)), types.isDataView(new DataView(new ArrayBuffer(2))), types.isTypedArray(new Uint8Array(2)), types.isUint8Array(Buffer.from('x')), types.isNativeError(new TypeError('x')), types.isBoxedPrimitive(Object(4)), types.isAsyncFunction(async function() {}), types.isGeneratorFunction(function*() {}), types.isProxy(new Proxy({}, {}))]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_util_types_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseUtilTypes = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseUtilTypes").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,true,true,true,true,true,true,true,true,true,true,true,true,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn node_constants_are_shared_with_filesystem_constants() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_constants");
    fs::write(dir.join("index.js"), "var constants = require('node:constants'); var fs = require('node:fs'); module.exports = function () { return [constants === fs.constants, constants.F_OK, constants.R_OK, constants.W_OK, constants.X_OK, constants.O_CREAT, constants.O_APPEND, constants.S_IFREG, constants.S_IFDIR, constants.COPYFILE_FICLONE_FORCE]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_constants_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 5);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseConstants = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseConstants").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,0,4,2,1,64,1024,32768,16384,4]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readline_splits_lines_and_supports_callback_and_promise_questions() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readline");
    fs::write(dir.join("index.js"), "var readline = require('node:readline'); var promiseReadline = require('node:readline/promises'); module.exports = async function () { var writes = []; var output = { write: function(value) { writes.push(String(value)); return true; } }; var rl = readline.createInterface({ output: output, historySize: 2, removeHistoryDuplicates: true }); var lines = []; rl.on('line', function(line) { lines.push(line); }); var callbackAnswer = new Promise(function(resolve) { rl.question('name? ', resolve); }); rl.write('alice\\nnext\\r\\n'); var answer = await callbackAnswer; rl.setPrompt('ready> '); rl.prompt(); rl.pause(); rl.resume(); readline.cursorTo(output, 2, 3); readline.clearLine(output, 0); rl.close(); var prl = promiseReadline.createInterface({ output: output }); var promised = prl.question('age? '); prl.write('42\\n'); var age = await promised; var iterator = prl[Symbol.asyncIterator](); var next = iterator.next(); prl.write('tail\\n'); var iterated = await next; prl.close(); return [answer, lines, rl.history, rl.closed, age, iterated.value, writes, readline.ReadLine === readline.Interface, prl instanceof promiseReadline.Interface]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_readline_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadline = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseReadline").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["alice",["alice","next"],["next","alice"],true,"42","42",["\u001b[4;3H","\u001b[2K"],true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn readline_questions_write_to_configured_output() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_readline_output");
    fs::write(dir.join("index.js"), "var readline = require('node:readline'); var promises = require('node:readline/promises'); module.exports = async function () { var writes = []; var output = { write: function(value) { writes.push(String(value)); return true; } }; var input = { on: function() {}, off: function() {} }; var callbackInterface = readline.createInterface({ input: input, output: output }); var callbackAnswer = new Promise(function(resolve) { callbackInterface.question('first? ', resolve); }); callbackInterface.write('yes\\n'); var first = await callbackAnswer; var promiseInterface = promises.createInterface({ input: input, output: output }); var secondPromise = promiseInterface.question('second? '); promiseInterface.write('ok\\n'); var second = await secondPromise; callbackInterface.close(); promiseInterface.close(); return [first, second, writes]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_readline_output_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseReadlineOutput = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseReadlineOutput").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["yes","ok",["first? ","second? "]]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn dns_modules_resolve_local_names_and_report_not_found() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_dns");
    fs::write(dir.join("index.js"), "var dns = require('node:dns'); var promises = require('node:dns/promises'); module.exports = async function () { var callbackLookup = await new Promise(function(resolve, reject) { dns.lookup('localhost', { family: 4 }, function(error, address, family) { error ? reject(error) : resolve([address, family]); }); }); var all = await dns.promises.lookup('localhost', { all: true }); var ipv6 = await promises.resolve6('localhost'); var reverse = await promises.reverse('127.0.0.1'); var errorCode; try { await promises.lookup('does-not-exist.invalid'); } catch (error) { errorCode = [error.code, error.syscall, error.hostname]; } dns.setDefaultResultOrder('ipv4first'); var resolver = new promises.Resolver(); resolver.setServers(['127.0.0.1']); return [callbackLookup, all, ipv6, reverse, errorCode, promises.getDefaultResultOrder(), resolver.getServers()]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_dns_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDns = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseDns").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["127.0.0.1",4],[{"address":"127.0.0.1","family":4},{"address":"::1","family":6}],["::1"],["localhost"],["ENOTFOUND","getaddrinfo","does-not-exist.invalid"],"ipv4first",["127.0.0.1"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn net_builtin_validates_addresses_and_block_lists() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_net");
    fs::write(dir.join("index.js"), "var net = require('node:net'); module.exports = function () { var block = new net.BlockList(); block.addAddress('127.0.0.1'); block.addRange('10.0.0.2', '10.0.0.5'); block.addSubnet('192.168.1.0', 24); block.addAddress('::1', 'ipv6'); var ipv4 = new net.SocketAddress({ address: '127.0.0.1', port: 8080 }); var ipv6 = new net.SocketAddress({ address: '::1', family: 'ipv6' }); var invalidPort = false; try { new net.SocketAddress({ address: '127.0.0.1', port: 70000 }); } catch (error) { invalidPort = error instanceof RangeError; } return [net.isIP('127.0.0.1'), net.isIP('2001:db8::1'), net.isIP('999.0.0.1'), net.isIPv4('01.2.3.4'), net.isIPv6('::ffff:192.0.2.1'), block.check('127.0.0.1'), block.check('10.0.0.4'), block.check('10.0.0.9'), block.check('192.168.1.88'), block.check('::1'), ipv4.toJSON(), ipv6.family, invalidPort]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_net_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNet = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNet").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[4,6,0,false,true,true,true,false,true,true,{"address":"127.0.0.1","port":8080,"family":"ipv4","flowlabel":0},"ipv6",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn net_socket_supports_write_batching_methods() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_net_write_batching");
    fs::write(
        dir.join("index.js"),
        "var net = require('node:net'); module.exports = function () { var socket = new net.Socket(); return [socket.cork() === socket, socket.uncork() === socket]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_net_write_batching_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let source = CString::new(format!(
        "globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetWriteBatching = module.exports;"
    ))
    .unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNetWriteBatching").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    assert_eq!(
        unsafe { CStr::from_ptr(result) }.to_string_lossy(),
        "[true,true]"
    );
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(empty_node_modules);
}

#[test]
fn net_socket_exchanges_bytes_with_a_real_tcp_peer() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
        assert_eq!(request, b"ping");
        stream.write_all(b"pong").unwrap();
    });

    let dir = temp_registry("builtin_net_socket");
    fs::write(dir.join("index.js"), "var net = require('node:net'); module.exports = async function (port) { var events = []; var socket = new net.Socket(); socket.on('connect', function() { events.push('connect'); }); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); }); socket.on('end', function() { events.push('end'); }); var completed = new Promise(function(resolve, reject) { socket.on('error', reject); socket.on('close', function(hadError) { events.push('close:' + hadError); resolve(); }); }); socket.connect(port, '127.0.0.1'); socket.end('ping'); await completed; return [events, socket.bytesWritten, socket.bytesRead, socket.destroyed, socket.remotePort, socket.remoteFamily]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_net_socket_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetSocket = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNetSocket").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        format!(r#"[["connect","data:pong","end","close:false"],4,4,true,{port},"IPv4"]"#)
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn net_socket_reads_while_its_write_side_remains_open() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(b"hello").unwrap();
        let mut reply = [0; 4];
        stream.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"ping");
        stream.write_all(b"world").unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
    });

    let dir = temp_registry("builtin_net_full_duplex_socket");
    fs::write(dir.join("index.js"), "var net = require('node:net'); module.exports = async function (port) { var chunks = [], socket = net.connect(port, '127.0.0.1'); var completed = new Promise(function(resolve, reject) { socket.on('error', reject); socket.on('data', function(chunk) { var value = chunk.toString(); chunks.push(value); if (value === 'hello') socket.write('ping'); }); socket.on('close', resolve); }); await completed; return [chunks, socket.bytesWritten, socket.bytesRead]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_net_full_duplex_socket_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let source = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetFullDuplex = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNetFullDuplex").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    assert_eq!(
        unsafe { CStr::from_ptr(result) }.to_string_lossy(),
        r#"[["hello","world"],4,10]"#
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(empty_node_modules);
}

#[test]
fn net_socket_timeout_fires_without_closing_the_connection() {
    use std::ffi::{CStr, CString};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
    });

    let dir = temp_registry("builtin_net_socket_timeout");
    fs::write(dir.join("index.js"), "var net = require('node:net'); module.exports = async function (port) { var socket = net.connect(port, '127.0.0.1'); return new Promise(function(resolve, reject) { socket.on('error', reject); socket.setTimeout(10, function() { var open = !socket.destroyed && socket.readable && socket.writable; socket.destroy(); resolve([open, socket.timeout]); }); }); };").unwrap();
    let empty_node_modules = temp_registry("builtin_net_socket_timeout_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let source = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetTimeout = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(
        CString::new("exerciseNetTimeout").unwrap().as_ptr(),
        CString::new(format!("[{port}]")).unwrap().as_ptr(),
    );
    assert_eq!(
        unsafe { CStr::from_ptr(result) }.to_string_lossy(),
        "[true,10]"
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(empty_node_modules);
}

/// A real npm package's own internal WebSocket close handshake (`ws`)
/// calls `stream.resume()` on the underlying `net.Socket` defensively
/// after tearing it down, and separately reads `this._readableState.
/// endEmitted`/`.length` directly (real Node's own internal stream
/// introspection, which several packages depend on beyond just the
/// public `.pause()`/`.resume()`/`.isPaused()` API) -- neither existed
/// on thaw's `net.Socket` at all: `.pause`/`.resume` weren't declared
/// (`call to undeclared function 'resume'`), and `_readableState` was
/// `undefined` (`cannot read property 'endEmitted' of undefined`).
/// Confirms `.pause()` actually suppresses `'data'` events (not a
/// no-op) and that `_readableState.endEmitted` becomes `true` only
/// once `'end'` has actually fired, matching real Node.
#[test]
fn net_socket_pause_and_resume_gate_data_events_and_track_readable_state() {
    use std::ffi::{CStr, CString};
    use std::io::Write;
    use std::net::{Shutdown, TcpListener};

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        stream.write_all(b"hello").unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
    });

    let dir = temp_registry("builtin_net_socket_pause_resume");
    fs::write(
        dir.join("index.js"),
        "var net = require('node:net'); module.exports = function (port) { return new Promise(function(resolve, reject) { var events = [], gotData = false; var socket = net.connect(port, '127.0.0.1'); socket.on('error', reject); socket.on('connect', function() { events.push('endEmittedBeforeData:' + socket._readableState.endEmitted); socket.pause(); events.push('isPaused:' + socket.isPaused()); }); socket.on('data', function() { gotData = true; }); setTimeout(function() { events.push('gotDataWhilePaused:' + gotData); socket.resume(); events.push('isPausedAfterResume:' + socket.isPaused()); }, 40); socket.on('end', function() { events.push('gotDataAfterResume:' + gotData); events.push('endEmittedAfterEnd:' + socket._readableState.endEmitted); resolve(events); }); }); };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_net_socket_pause_resume_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let source = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetPauseResume = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(
        CString::new("exerciseNetPauseResume").unwrap().as_ptr(),
        CString::new(format!("[{port}]")).unwrap().as_ptr(),
    );
    assert_eq!(
        unsafe { CStr::from_ptr(result) }.to_string_lossy(),
        concat!(
            r#"["endEmittedBeforeData:false","isPaused:true","#,
            r#""gotDataWhilePaused:false","isPausedAfterResume:false","#,
            r#""gotDataAfterResume:true","endEmittedAfterEnd:true"]"#
        )
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(empty_node_modules);
}

#[test]
fn http_client_request_timeout_fires() {
    use std::ffi::{CStr, CString};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (_stream, _) = listener.accept().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(50));
    });
    let dir = temp_registry("builtin_http_request_timeout");
    fs::write(dir.join("index.js"), "var http = require('node:http'); module.exports = function(port) { return new Promise(function(resolve, reject) { var request = http.get({ hostname: '127.0.0.1', port: port }); request.on('error', reject); request.setTimeout(10, function() { request.destroy(); resolve(request.timeout); }); }); };").unwrap();
    let empty_node_modules = temp_registry("builtin_http_request_timeout_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let source = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpTimeout = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(
        CString::new("exerciseHttpTimeout").unwrap().as_ptr(),
        CString::new(format!("[{port}]")).unwrap().as_ptr(),
    );
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "10");
    server.join().unwrap();
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(empty_node_modules);
}

#[test]
fn http_client_requests_and_parses_a_real_chunked_response() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request.starts_with("GET /items?q=thaw HTTP/1.1\r\n"));
        assert!(request.to_ascii_lowercase().contains("x-thaw: enabled\r\n"));
        stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nX-Reply: yes\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nConnection: close\r\n\r\n4\r\nthaw\r\n3\r\n-ok\r\n0\r\n\r\n",
                )
                .unwrap();
    });

    let dir = temp_registry("builtin_http_client");
    fs::write(
            dir.join("index.js"),
            "var http = require('node:http'); module.exports = async function (port) { return await new Promise(function(resolve, reject) { var request = http.get({ hostname: '127.0.0.1', port: port, path: '/items?q=thaw', headers: { 'X-Thaw': 'enabled' } }, function(response) { var chunks = []; response.setEncoding('utf8'); response.on('data', function(chunk) { chunks.push(chunk); }); response.on('end', function() { resolve([response.statusCode, response.statusMessage, response.httpVersion, response.headers['x-reply'], response.headers['set-cookie'], response.rawHeaders.length, chunks.join(''), response.complete, request.finished, request.destroyed, http.METHODS.indexOf('GET') >= 0, http.STATUS_CODES[200]]); }); }); request.on('error', reject); }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http_client_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 5);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpClient = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpClient").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[200,"OK","1.1","yes",["a=1","b=2"],10,"thaw-ok",true,true,false,true,"OK"]"#
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_client_rejects_truncated_bodies_but_accepts_unframed_eof() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        for _ in 0..5 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let request = String::from_utf8(request).unwrap();
            let packet: &[u8] = if request.starts_with("GET /length ") {
                b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\nabc"
            } else if request.starts_with("GET /chunk ") {
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n"
            } else if request.starts_with("GET /trailers ") {
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n0\r\n"
            } else if request.starts_with("GET /unframed ") {
                b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabc"
            } else {
                assert!(request.starts_with("GET /normal "));
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n0\r\n\r\n"
            };
            stream.write_all(packet).unwrap();
        }
    });

    let dir = temp_registry("builtin_http_truncated_eof");
    fs::write(
        dir.join("index.js"),
        r#"var http = require('node:http'); module.exports = async function(port) {
          function probe(path) { return new Promise(function(resolve) {
            var events = [], request = http.get({ hostname: '127.0.0.1', port: port, path: path }, function(response) {
              events.push('headers:' + response.complete);
              response.on('data', function(value) { events.push('data:' + value.toString()); });
              response.on('end', function() { events.push('end:' + response.complete); });
              response.on('aborted', function() { events.push('aborted:' + response.complete); });
              response.on('error', function(error) { events.push('response-error:' + error.code); });
              response.on('close', function() { events.push('response-close'); });
            });
            request.on('error', function(error) { events.push('request-error:' + error.code); });
            request.on('close', function() { events.push('request-close'); resolve(events); });
          }); }
          return [await probe('/length'), await probe('/chunk'), await probe('/trailers'), await probe('/unframed'), await probe('/normal')];
        };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_truncated_eof_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpTruncatedEof = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpTruncatedEof").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    let cases: Vec<Vec<String>> = serde_json::from_str(&result).unwrap();
    assert_eq!(cases.len(), 5);
    let failure_tail = [
        "aborted:false",
        "response-error:ECONNRESET",
        "response-close",
        "request-error:ECONNRESET",
        "request-close",
    ];
    for events in &cases[..3] {
        assert_eq!(events.first().map(String::as_str), Some("headers:false"));
        let aborted = events.iter().position(|event| event == "aborted:false").unwrap();
        let prefix = events[1..aborted]
            .iter()
            .map(|event| event.strip_prefix("data:").unwrap())
            .collect::<Vec<_>>()
            .join("");
        assert!("abc".starts_with(&prefix));
        assert_eq!(events[aborted..].iter().map(String::as_str).collect::<Vec<_>>(), failure_tail);
    }
    for events in &cases[3..] {
        assert_eq!(events.first().map(String::as_str), Some("headers:false"));
        let ended = events.iter().position(|event| event == "end:true").unwrap();
        let body = events[1..ended]
            .iter()
            .map(|event| event.strip_prefix("data:").unwrap())
            .collect::<Vec<_>>()
            .join("");
        assert_eq!(body, "abc");
        assert_eq!(events[ended..].iter().map(String::as_str).collect::<Vec<_>>(), ["end:true", "request-close"]);
    }
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_client_truncated_eof_keeps_cleanup_after_listener_reentry() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_truncated_reentry");
    fs::write(
        dir.join("index.js"),
        r#"var http = require('node:http'), EventEmitter = require('node:events'); module.exports = function() {
          var socket = new EventEmitter(), events = [], marker = new Error('listener marker'), caught = false;
          socket.write = function() { return true; }; socket.end = function() { return this; };
          socket.destroy = function() { this.emit('close'); return this; };
          var request = http.request({ hostname: 'example.test', port: 80, agent: false, _transport: { createConnection: function() { return socket; } } }, function(response) {
            events.push('headers:' + response.complete);
            response.on('data', function() { events.push('data'); });
            response.on('end', function() { events.push('end'); });
            response.on('aborted', function() { events.push('aborted:' + arguments.length); request.destroy(); throw marker; });
            response.on('error', function(error) { events.push('response-error:' + error.code); });
            response.on('close', function() { events.push('response-close:' + arguments.length); });
          });
          request.on('error', function(error) { events.push('request-error:' + error.code); });
          request.on('close', function() { events.push('request-close'); });
          request.end(); socket.emit('connect');
          socket.emit('data', Buffer.from('HTTP/1.1 200 OK\r\nContent-Length: 8\r\n\r\nabc'));
          try { socket.emit('end'); } catch (error) { caught = error === marker; }
          return Promise.resolve().then(function() { return [caught, events]; });
        };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_truncated_reentry_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpTruncatedReentry = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpTruncatedReentry").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,["headers:false","aborted:0","response-error:ECONNRESET","response-close:0","request-error:ECONNRESET","request-close"]]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_client_parse_failure_keeps_first_notification_exception_when_destroy_throws() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_parse_failure_cleanup");
    fs::write(
        dir.join("index.js"),
        r#"var http = require('node:http'), EventEmitter = require('node:events'); module.exports = function() {
          var socket = new EventEmitter(), events = [], notification = new Error('notification marker'), destruction = new Error('destroy marker'), caught = false;
          socket.write = function() { return true; }; socket.end = function() { return this; };
          socket.on('close', function() { events.push('socket-close'); throw destruction; });
          socket.destroy = function() { events.push('destroy'); this.emit('close'); return this; };
          var request = http.request({ hostname: 'example.test', port: 80, agent: false, _transport: { createConnection: function() { return socket; } } });
          request.on('error', function() { events.push('request-error'); throw notification; });
          request.on('close', function() { events.push('request-close:' + arguments.length); });
          request.end(); socket.emit('connect');
          try { socket.emit('data', Buffer.from('invalid response\r\n\r\n')); } catch (error) { caught = error === notification; }
          return Promise.resolve().then(function() { return [caught, events]; });
        };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_parse_failure_cleanup_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpParseFailureCleanup = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpParseFailureCleanup").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,["request-error","destroy","socket-close","request-close:0"]]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_response_complete_waits_for_chunked_body_end() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nbody\r\n0\r\n\r\n").unwrap();
    });

    let dir = temp_registry("builtin_http_complete_after_body");
    fs::write(
        dir.join("index.js"),
        r#"var http = require('node:http'); module.exports = function(port) { return new Promise(function(resolve, reject) {
          var request = http.get({ hostname: '127.0.0.1', port: port }, function(response) {
            var atHeaders = response.complete, body = [];
            response.on('data', function(chunk) { body.push(chunk.toString()); });
            response.on('end', function() { resolve([atHeaders, body.join(''), response.complete]); });
          }); request.on('error', reject);
        }); };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_complete_after_body_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpCompleteAfterBody = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpCompleteAfterBody").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[false,"body",true]"#);
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_incoming_pause_buffers_chunked_body_and_resume_waits_for_end() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\na\r\n1\r\nb\r\n0\r\n\r\n").unwrap();
    });

    let dir = temp_registry("builtin_http_incoming_pause");
    fs::write(
        dir.join("index.js"),
        r#"var http = require('node:http'); module.exports = function(port) { return new Promise(function(resolve, reject) {
          var request = http.get({ hostname: '127.0.0.1', port: port }, function(response) {
            var events = [], pausedSnapshot, endedBeforeBody;
            response.resume(); endedBeforeBody = !!response.readableEnded;
            response.on('data', function(chunk) { events.push(chunk.toString()); if (events.length === 1) { response.pause(); queueMicrotask(function() { pausedSnapshot = events.slice(); response.resume(); }); } });
            response.on('end', function() { events.push('end'); response.resume(); setTimeout(function() { resolve([endedBeforeBody, pausedSnapshot, events, response.readableEnded]); }, 0); });
          }); request.on('error', reject);
        }); };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_incoming_pause_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpIncomingPause = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpIncomingPause").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[false,["a"],["a","b","end"],true]"#);
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_incoming_decodes_split_utf8_and_flushes_eof_before_end() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        for case in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            if case == 0 {
                stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n1\r\nA\r\n1\r\n\xe9\r\n1\r\n\x9b\r\n1\r\n\xaa\r\n0\r\n\r\n").unwrap();
            } else {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nConnection: close\r\n\r\n\xe9").unwrap();
            }
        }
    });

    let dir = temp_registry("builtin_http_incoming_decoder");
    fs::write(
        dir.join("index.js"),
        r#"var http = require('node:http'); module.exports = async function(port) {
          function get(path, pauseFirst) { return new Promise(function(resolve, reject) {
            var request = http.get({ hostname: '127.0.0.1', port: port, path: path }, function(response) {
              var events = []; response.setEncoding('utf8');
              response.on('data', function(chunk) {
                events.push(chunk);
                if (pauseFirst && chunk === 'A') { response.pause(); queueMicrotask(function() { response.resume(); }); }
              });
              response.on('end', function() { events.push('end'); resolve(events); });
            }); request.on('error', reject);
          }); }
          var split = await get('/split', true), eof = await get('/eof', false);
          var direct = new http.IncomingMessage({ destroy: function() {} }), switched = [];
          direct.setEncoding('utf8');
          direct.on('data', function(chunk) { switched.push(chunk); if (chunk === 'A') direct.setEncoding('hex'); });
          direct.on('end', function() { switched.push('end'); });
          direct._queueBody(Buffer.from([65, 233])); direct._queueBody(Buffer.from([66])); direct._finishBody();
          await new Promise(function(resolve) { direct.once('end', resolve); });
          return [split, eof, switched];
        };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_incoming_decoder_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpIncomingDecoder = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpIncomingDecoder").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[["A","雪","end"],["�","end"],["A","�","42","end"]]"#);
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_incoming_listener_variants_start_data_and_end_drain() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_incoming_listener_variants");
    fs::write(
        dir.join("index.js"),
        r#"var http = require('node:http'); module.exports = function() {
          function probe(method) { return new Promise(function(resolve) {
            var message = new http.IncomingMessage({ destroy: function() {} }), chunks = [];
            message[method]('data', function(chunk) { chunks.push(chunk.toString()); });
            message.once('end', function() { chunks.push('end'); resolve(chunks); });
            message._queueBody(Buffer.from('a')); message._finishBody();
          }); }
          return Promise.all(['on', 'addListener', 'once', 'prependListener', 'prependOnceListener'].map(probe));
        };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_incoming_listener_variants_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpIncomingListeners = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpIncomingListeners").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[["a","end"],["a","end"],["a","end"],["a","end"],["a","end"]]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_incoming_eof_orders_data_end_before_request_close() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nbody").unwrap();
    });

    let dir = temp_registry("builtin_http_incoming_eof_order");
    fs::write(
        dir.join("index.js"),
        r#"var http = require('node:http'); module.exports = function(port) { return new Promise(function(resolve, reject) {
          var events = [], request = http.get({ hostname: '127.0.0.1', port: port }, function(response) {
            response.on('data', function(chunk) { events.push('data:' + chunk.toString()); });
            response.on('end', function() { events.push('end'); });
          });
          request.on('error', reject);
          request.on('close', function() { events.push('close'); setTimeout(function() { resolve(events); }, 0); });
        }); };"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_incoming_eof_order_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpIncomingEof = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpIncomingEof").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["data:body","end","close"]"#);
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_client_upgrades_and_keeps_the_socket_duplex() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        assert!(String::from_utf8(request)
            .unwrap()
            .to_ascii_lowercase()
            .contains("upgrade: websocket\r\n"));
        stream
            .write_all(b"HTTP/1.1 101 Switching Protocols\r\nConnection: Upgrade\r\nUpgrade: websocket\r\n\r\nhead")
            .unwrap();
        let mut reply = [0; 4];
        stream.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"ping");
        stream.write_all(b"pong").unwrap();
    });

    let dir = temp_registry("builtin_http_client_upgrade");
    fs::write(dir.join("index.js"), "var http = require('node:http'); module.exports = async function (port) { return new Promise(function(resolve, reject) { var request = http.request({ hostname: '127.0.0.1', port: port, path: '/', headers: { Connection: 'Upgrade', Upgrade: 'websocket' } }); request.on('error', reject); request.on('response', function() { reject(new Error('unexpected response')); }); request.on('upgrade', function(response, socket, head) { var result = [response.statusCode, response.complete, head.toString()]; socket.on('data', function(data) { result.push(data.toString()); socket.destroy(); resolve(result); }); socket.write('ping'); }); request.end(); }); };").unwrap();
    let empty_node_modules = temp_registry("builtin_http_client_upgrade_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let source = CString::new(format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpUpgrade = module.exports;")).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpUpgrade").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    assert_eq!(
        unsafe { CStr::from_ptr(result) }.to_string_lossy(),
        r#"[101,true,"head","pong"]"#
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(empty_node_modules);
}

#[test]
fn http_client_emits_informational_responses_and_parses_trailers() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut byte = [0];
        while !request.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
        }
        stream
                .write_all(b"HTTP/1.1 103 Early Hints\r\nLink: </style.css>; rel=preload\r\n\r\nHTTP/1.1 100 Continue\r\nX-Interim: yes\r\n\r\nHTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nTrailer: X-Check, Set-Cookie\r\nConnection: close\r\n\r\n5\r\nhello\r\n0\r\nX-Check: one\r\nX-Check: two\r\nSet-Cookie: t=1\r\n\r\n")
                .unwrap();
    });

    let dir = temp_registry("builtin_http_information_trailers");
    fs::write(
            dir.join("index.js"),
            "var http = require('node:http'); module.exports = async function (port) { return new Promise(function(resolve, reject) { var information = [], continued = 0, request = http.get({ hostname: '127.0.0.1', port: port, path: '/' }, function(response) { var chunks = []; response.on('data', function(chunk) { chunks.push(chunk); }); response.on('end', function() { resolve([information, continued, Buffer.concat(chunks).toString(), response.trailers['x-check'], response.trailers['set-cookie'], response.trailersDistinct['x-check'], response.rawTrailers.length, response.complete]); }); }); request.on('information', function(info) { information.push([info.statusCode, info.statusMessage, info.headers.link || info.headers['x-interim'], info.httpVersionMajor, info.httpVersionMinor]); }); request.on('continue', function() { continued++; }); request.on('error', reject); }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http_information_trailers_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 5);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpInformation = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpInformation").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[[103,"Early Hints","</style.css>; rel=preload",1,1],[100,"Continue","yes",1,1]],1,"hello","one, two",["t=1"],["one","two"],6,true]"#
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn global_fetch_sends_requests_follows_redirects_and_returns_responses() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut brotli = brotli::CompressorWriter::new(Vec::new(), 4096, 5, 22);
    brotli.write_all(b"compressed").unwrap();
    let brotli = brotli.into_inner();
    let mut stacked = brotli::CompressorWriter::new(Vec::new(), 4096, 5, 22);
    stacked.write_all(&[
                            0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x4b, 0xce,
                            0xcf, 0x2d, 0x28, 0x4a, 0x2d, 0x2e, 0x4e, 0x4d, 0x01, 0x00, 0x1e, 0x4b,
                            0x56, 0x97, 0x0a, 0x00, 0x00, 0x00,
                        ]).unwrap();
    let stacked = stacked.into_inner();
    let server = std::thread::spawn(move || {
        fn read_request(stream: &mut std::net::TcpStream) -> Vec<u8> {
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            let head = String::from_utf8_lossy(&request);
            let length = head
                .lines()
                .find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length: ").map(str::to_owned))
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            let start = request.len();
            request.resize(start + length, 0);
            stream.read_exact(&mut request[start..]).unwrap();
            request
        }

        for expected in [
            "GET /integrity-ok ",
            "GET /integrity-bad ",
            "GET /integrity-strong ",
            "GET /integrity-alternative ",
            "GET /integrity-unknown ",
            "GET /integrity-sha512 ",
            "HEAD /integrity-head ",
            "GET /integrity-abort ",

            "POST /stream-redirect ",
            "POST /replay-redirect ",
            "POST /replay-final ",
            "GET /redirect ",
            "GET /final ",
            "POST /echo ",
            "POST /multipart ",
            "POST /post-redirect ",
            "GET /post-final ",
            "GET /gzip ",
            "GET /deflate ",
            "GET /br ",
            "GET /stacked ",
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_request(&mut stream);
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with(expected), "{request}");
            if expected.contains("/integrity-") {
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nintegrity").unwrap();
            } else if expected.contains("stream-redirect") || expected.contains("replay-redirect") {
                stream.write_all(b"HTTP/1.1 307 Temporary Redirect\r\nLocation: /replay-final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            } else if expected.contains("replay-final") {
                assert!(request.ends_with("replay"));
                stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nreplay").unwrap();
            } else if expected.contains("post-redirect") {
                assert!(request.ends_with("again"));
                stream
                        .write_all(b"HTTP/1.1 302 Found\r\nLocation: /post-final#ignored\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .unwrap();
            } else if expected.contains("redirect") {
                stream
                        .write_all(b"HTTP/1.1 303 See Other\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .unwrap();
            } else if expected == "GET /final " {
                assert!(request.to_ascii_lowercase().contains("content-type: text/plain\r\n"), "{request}");
                stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: a=1\r\nSet-Cookie: b=2\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}")
                        .unwrap();
            } else if expected.contains("echo") {
                assert!(request.to_ascii_lowercase().contains("x-thaw: enabled\r\n"));
                assert!(request.ends_with("payload"));
                stream
                        .write_all(b"HTTP/1.1 201 Created\r\nContent-Type: text/plain\r\nContent-Length: 7\r\nConnection: close\r\n\r\npayload")
                        .unwrap();
            } else if expected.contains("multipart") {
                let lower = request.to_ascii_lowercase();
                assert!(lower
                    .contains("content-type: multipart/form-data; boundary=----thaw-formdata-"));
                assert!(request.contains("name=\"title\"\r\n\r\nthaw"));
                assert!(request.contains("name=\"asset\"; filename=\"note.txt\""));
                assert!(request.ends_with("file-body\r\n------thaw-formdata-1--\r\n"));
                stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: multipart/form-data; boundary=reply-boundary\r\nConnection: close\r\n\r\n--reply-boundary\r\nContent-Disposition: form-data; name=\"answer\"\r\n\r\n42\r\n--reply-boundary\r\nContent-Disposition: form-data; name=\"upload\"; filename=\"reply.txt\"\r\nContent-Type: text/plain\r\n\r\nreply-body\r\n--reply-boundary--\r\n")
                        .unwrap();
            } else if expected.contains("gzip")
                || expected.contains("deflate")
                || expected.contains("/br")
                || expected.contains("/stacked")
            {
                assert!(request
                    .to_ascii_lowercase()
                    .contains("accept-encoding: gzip, deflate, br\r\n"));
                let (encoding, compressed): (&str, &[u8]) = if expected.contains("gzip") {
                    (
                        "gzip",
                        &[
                            0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0x4b, 0xce,
                            0xcf, 0x2d, 0x28, 0x4a, 0x2d, 0x2e, 0x4e, 0x4d, 0x01, 0x00, 0x1e, 0x4b,
                            0x56, 0x97, 0x0a, 0x00, 0x00, 0x00,
                        ],
                    )
                } else if expected.contains("deflate") {
                    (
                        "deflate",
                        &[
                            0x78, 0x9c, 0x4b, 0xce, 0xcf, 0x2d, 0x28, 0x4a, 0x2d, 0x2e, 0x4e, 0x4d,
                            0x01, 0x00, 0x17, 0x3f, 0x04, 0x36,
                        ],
                    )
                } else if expected.contains("/stacked") {
                    ("GZip, BR", &stacked)
                } else {
                    ("br", &brotli)
                };
                write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Encoding: {encoding}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        compressed.len()
                    )
                    .unwrap();
                stream.write_all(compressed).unwrap();
            } else {
                assert!(!request.to_ascii_lowercase().contains("content-type:"));
                assert!(!request.to_ascii_lowercase().contains("content-length:"));
                stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nrewritten")
                        .unwrap();
            }
        }
    });

    let dir = temp_registry("global_fetch");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function (port) { var base = 'http://127.0.0.1:' + port; var httpForPressure = require('node:http'), EventEmitterForPressure = require('node:events').EventEmitter, savedRequestForPressure = httpForPressure.request, incomingForPressure, pausesForPressure = 0, resumesForPressure = 0; httpForPressure.request = function(url, options, callback) { var requestForPressure = new EventEmitterForPressure(), incoming = new EventEmitterForPressure(), sent = 0, paused = false, destroyed = false; incomingForPressure = incoming; incoming.statusCode = 200; incoming.headers = {}; incoming.statusMessage = 'OK'; function pump() { while (!paused && !destroyed && sent < 2) { sent++; incoming.emit('data', new Uint8Array(16384)); } if (sent === 2 && !paused && !destroyed) { destroyed = true; incoming.emit('end'); } } incoming.pause = function() { paused = true; pausesForPressure++; }; incoming.resume = function() { paused = false; resumesForPressure++; queueMicrotask(pump); }; incoming.destroy = function() { destroyed = true; }; requestForPressure.write = function() {}; requestForPressure.destroy = incoming.destroy; requestForPressure.end = function() { queueMicrotask(function() { callback(incoming); pump(); }); }; return requestForPressure; }; try { var boundedResponse = await fetch(base + '/mock-pressure'); if (pausesForPressure !== 1) throw new Error('fetch did not pause at byte capacity'); var boundedBytes = await boundedResponse.bytes(); if (boundedBytes.length !== 32768 || pausesForPressure < 1 || resumesForPressure < 1) throw new Error('fetch did not resume its body'); var cancelledResponse = await fetch(base + '/mock-cancel'); await cancelledResponse.body.cancel(); incomingForPressure.emit('data', new Uint8Array(1)); incomingForPressure.emit('end'); } finally { httpForPressure.request = savedRequestForPressure; } var uploadController = new AbortController(), uploadReason = {}, cancelled = false, uploadRejected = false; var uploadBody = new ReadableStream({ pull: function() { queueMicrotask(function() { uploadController.abort(uploadReason); }); }, cancel: function(reason) { cancelled = reason === uploadReason; return new Promise(function() {}); } }); try { await fetch(base + '/never-upload', { method: 'POST', body: uploadBody, duplex: 'half', signal: uploadController.signal }); } catch (error) { uploadRejected = error === uploadReason; } if (!uploadRejected || !cancelled) throw new Error('upload abort did not cancel and reject'); var integrityResponse = await fetch(base + '/integrity-ok', { integrity: 'sha256-eFh8Qe2ZozdQItwovogvcrGmCKDax6qQDGH0iyuze+Y=?ignored' }); if (integrityResponse.bodyUsed || await integrityResponse.text() !== 'integrity') throw new Error('integrity response body changed'); async function integrityMustReject(path, metadata) { var rejected = false; try { await fetch(base + path, { integrity: metadata }); } catch (error) { rejected = error instanceof TypeError; } if (!rejected) throw new Error('integrity mismatch accepted'); } await integrityMustReject('/integrity-bad', 'sha256-invalid'); await integrityMustReject('/integrity-strong', 'sha256-eFh8Qe2ZozdQItwovogvcrGmCKDax6qQDGH0iyuze+Y= sha512-invalid'); var integrityAlternative = await fetch(base + '/integrity-alternative', { integrity: 'sha384-invalid sha384-tABW7Annv76lWwWhcvnWl/qsMR4eXipmMZ5mhm2LMV2od4nkRSN5ueNI58teuBOC' }); if (await integrityAlternative.text() !== 'integrity') throw new Error('integrity alternative failed'); var integrityUnknown = await fetch(base + '/integrity-unknown', { integrity: 'unknown-invalid' }); if (await integrityUnknown.text() !== 'integrity') throw new Error('unsupported integrity token changed body'); var integritySha512 = await fetch(base + '/integrity-sha512', { integrity: 'sha512-8fcC6+NM8Jwev3MWM1nfkzwmePdmxzdp2xpHFJccSSwqcZyyYNYVmnG4Z30Z5EvyGFBCM+ixLVYRCMEqZBB4dg==' }); if (await integritySha512.text() !== 'integrity') throw new Error('sha512 integrity failed'); var integrityHead = await fetch(base + '/integrity-head', { method: 'HEAD', integrity: 'sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=' }); if (integrityHead.body !== null) throw new Error('integrity HEAD gained body'); var digestController = new AbortController(), digestReason = {}, originalDigest = crypto.subtle.digest, digestRejected = false; crypto.subtle.digest = function(algorithm, bytes) { var result = originalDigest.call(this, algorithm, bytes); digestController.abort(digestReason); return result; }; try { await fetch(base + '/integrity-abort', { integrity: 'sha256-eFh8Qe2ZozdQItwovogvcrGmCKDax6qQDGH0iyuze+Y=', signal: digestController.signal }); } catch (error) { digestRejected = error === digestReason; } finally { crypto.subtle.digest = originalDigest; } if (!digestRejected) throw new Error('integrity abort did not preserve reason'); var streamRejected = false; try { await fetch(base + '/stream-redirect', { method: 'POST', body: new ReadableStream({ start: function(controller) { controller.close(); } }), duplex: 'half' }); } catch (error) { streamRejected = error instanceof TypeError; } if (!streamRejected) throw new Error('stream body was replayed'); var replaySource = new Request(base + '/replay-redirect', { method: 'POST', headers: { 'Content-Length': '6' }, body: 'replay' }); var replayResponse = await fetch(new Request(replaySource.clone())); if (await replayResponse.text() !== 'replay') throw new Error('ordinary body failed replay'); var redirected = await fetch(base + '/redirect', { headers: { 'Content-Type': 'text/plain' } }), cookies = redirected.headers.getSetCookie(), json = await redirected.json(); var posted = await fetch(new Request(base + '/echo', { method: 'POST', headers: { 'X-Thaw': 'enabled' }, body: 'payload' })), before = posted.bodyUsed, text = await posted.text(); var data = new FormData(); data.append('title', 'thaw'); data.append('asset', new Blob(['file-body'], { type: 'text/plain' }), 'note.txt'); var multipartRequest = new Request(base + '/multipart', { method: 'POST', body: data }), parsedRequest = await multipartRequest.clone().formData(), multipart = await fetch(multipartRequest), parsed = await multipart.formData(), upload = parsed.get('upload'); var rewritten = await fetch(base + '/post-redirect#source', { method: 'POST', body: 'again' }), rewrittenText = await rewritten.text(), gzip = await fetch(base + '/gzip', { integrity: 'sha256-naMIwuS8M6+nLfXAiLX8VnPEd/PvIda9qjWDk4NPmAQ=' }), gzipText = await gzip.text(), deflate = await fetch(base + '/deflate'), deflateText = await deflate.text(), br = await fetch(base + '/br'), brText = await br.text(), stacked = await fetch(base + '/stacked'), stackedText = await stacked.text(); var controller = new AbortController(), abortReason; controller.abort('stop'); try { await fetch(base + '/unused', { signal: controller.signal }); } catch (error) { abortReason = error; } var schemeError; try { await fetch('file:///tmp/value'); } catch (error) { schemeError = error instanceof TypeError; } return [redirected.status, redirected.ok, redirected.redirected, redirected.url, cookies, json.ok, posted.status, posted.statusText, posted.headers.get('content-type'), before, posted.bodyUsed, text, typeof fetch, abortReason, schemeError, parsedRequest.get('title'), await parsedRequest.get('asset').text(), parsed.get('answer'), upload instanceof File, upload.name, upload.type, await upload.text(), rewritten.redirected, rewritten.url, rewrittenText, gzipText, gzip.headers.get('content-encoding'), deflateText, deflate.headers.get('content-encoding'), brText, br.headers.get('content-encoding'), stackedText, stacked.headers.get('content-encoding')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("global_fetch_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 7);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseFetch = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseFetch").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        format!(
            r#"[200,true,true,"http://127.0.0.1:{port}/final",["a=1","b=2"],true,201,"Created","text/plain",false,true,"payload","function","stop",true,"thaw","file-body","42",true,"reply.txt","text/plain","reply-body",true,"http://127.0.0.1:{port}/post-final","rewritten","compressed","gzip","compressed","deflate","compressed","br","compressed","GZip, BR"]"#
        )
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn global_fetch_resolves_headers_before_delayed_body_chunks() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Duration;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        fn read_request(stream: &mut std::net::TcpStream) {
            let mut request = Vec::new();
            let mut byte = [0u8; 1];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
        }

        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\n")
            .unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(250));
        stream.write_all(b"one").unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(100));
        stream.write_all(b"two").unwrap();
        drop(stream);
        let (mut stream, _) = listener.accept().unwrap();
        read_request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\n")
            .unwrap();
        stream.flush().unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let _ = stream.write_all(b"late");
    });

    let dir = temp_registry("global_fetch_streaming");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function (port) { var base = 'http://127.0.0.1:' + port, started = Date.now(), response = await fetch(base + '/slow'), headersElapsed = Date.now() - started, reader = response.body.getReader(), first = await reader.read(), firstElapsed = Date.now() - started, second = await reader.read(), done = await reader.read(); var controller = new AbortController(), abortedResponse = await fetch(base + '/abort', { signal: controller.signal }), abortedReader = abortedResponse.body.getReader(), pending = abortedReader.read(), bodyReason; controller.abort('body-stop'); try { await pending; } catch (error) { bodyReason = error; } return [headersElapsed < 200, firstElapsed >= 200, new TextDecoder().decode(first.value), new TextDecoder().decode(second.value), done.done, response.bodyUsed, bodyReason]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("global_fetch_streaming_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamingFetch = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseStreamingFetch").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,true,"one","two",true,true,"body-stop"]"#);
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_agents_and_header_validators_share_across_https() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_http_agents");
    fs::write(
            dir.join("index.js"),
            "var http = require('node:http'), https = require('node:https'); module.exports = function () { var agent = new http.Agent({ keepAlive: true, maxSockets: 7, maxTotalSockets: 9 }); var secureAgent = new https.Agent({ keepAliveMsecs: 250 }); var request = http.request({ host: 'example.com', port: 8080, agent: agent }); var secureRequest = https.request({ host: 'example.com', agent: false }); request.setHeader('X-Valid', ['one', 'two']); var errors = []; try { http.validateHeaderName('bad name'); } catch (error) { errors.push(error.code); } try { https.validateHeaderValue('X-Test', 'bad\\nvalue'); } catch (error) { errors.push(error.code); } try { request.setHeader('X-Missing', undefined); } catch (error) { errors.push(error.code); } http.setMaxIdleHTTPParsers(10); return [agent instanceof http.Agent, secureAgent instanceof https.Agent, secureAgent instanceof http.Agent, request.agent === agent, secureRequest.agent, request.getHeader('x-valid'), agent.getName({ host: 'example.com', port: 8080, family: 4 }), agent.keepAlive, agent.maxSockets, agent.maxTotalSockets, agent.totalSocketCount, secureAgent.protocol, secureAgent.defaultPort, secureAgent.keepAliveMsecs, http.globalAgent instanceof http.Agent, https.globalAgent instanceof https.Agent, errors]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http_agents_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 7);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpAgents = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpAgents").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,true,true,null,["one","two"],"example.com:8080::4",true,7,9,0,"https:",443,250,true,true,["ERR_INVALID_HTTP_TOKEN","ERR_INVALID_CHAR","ERR_HTTP_INVALID_HEADER_VALUE"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn child_process_sync_apis_execute_and_report_node_shaped_results() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_child_process_sync");
    fs::write(
            dir.join("index.js"),
            r#"var cp = require('node:child_process'); module.exports = function (cwd) { var failed = cp.spawnSync('/bin/sh', ['-c', 'printf out; printf err >&2; exit 3'], { encoding: 'utf8', cwd: cwd }); var input = cp.spawnSync('/bin/sh', ['-c', 'cat'], { input: Buffer.from('stdin') }); var environment = cp.execFileSync('/bin/sh', ['-c', 'printf "$VALUE"'], { encoding: 'utf8', env: { VALUE: 'env-ok' } }); var shell = cp.execSync('printf shell-ok', { encoding: 'utf8' }); var missing = cp.spawnSync('/thaw/does-not-exist', []); var thrown; try { cp.execFileSync('/bin/sh', ['-c', 'printf bad >&2; exit 7'], { encoding: 'utf8' }); } catch (error) { thrown = [error.status, error.stderr, error.stdout, error.pid > 0]; } var overflow = cp.spawnSync('/bin/sh', ['-c', 'printf 12345'], { maxBuffer: 4 }); return [failed.status, failed.signal, failed.stdout, failed.stderr, failed.output[1], failed.pid > 0, Buffer.isBuffer(input.stdout), input.stdout.toString(), input.stderr.length, environment, shell, missing.status, missing.error.code, missing.error.path, thrown, overflow.status, overflow.error.code]; };"#,
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_child_process_sync_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildProcessSync = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseChildProcessSync").unwrap();
    let arguments =
        CString::new(serde_json::to_string(&[dir.to_string_lossy().into_owned()]).unwrap())
            .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[3,null,"out","err","out",true,true,"stdin",0,"env-ok","shell-ok",null,"ENOENT","/thaw/does-not-exist",[7,"bad","",true],null,"ENOBUFS"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn child_process_async_apis_stream_and_emit_lifecycle_events() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_child_process_async");
    fs::write(
            dir.join("index.js"),
            r#"var cp = require('node:child_process'); module.exports = async function () { var child = cp.spawn('/bin/sh', ['-c', 'read value; printf "out:$value"; printf "err:$value" >&2; exit 4']); var events = [], stdout = [], stderr = []; child.on('spawn', function() { events.push('spawn:' + (child.pid > 0)); }); child.stdout.on('data', function(value) { stdout.push(value.toString()); }); child.stderr.on('data', function(value) { stderr.push(value.toString()); }); var closed = new Promise(function(resolve, reject) { child.on('error', reject); child.on('exit', function(code, signal) { events.push('exit:' + code + ':' + signal); }); child.on('close', function(code, signal) { events.push('close:' + code + ':' + signal); resolve(); }); }); child.stdin.end('hello\n'); await closed; var executed = await new Promise(function(resolve) { cp.exec('printf callback', { encoding: 'utf8' }, function(error, out, err) { resolve([error, out, err]); }); }); var failed = await new Promise(function(resolve) { cp.execFile('/bin/sh', ['-c', 'printf failure >&2; exit 6'], { encoding: 'utf8' }, function(error, out, err) { resolve([error.code, error.stderr, out, err]); }); }); var killed = cp.spawn('/bin/sh', ['-c', 'sleep 10']); var killedResult = new Promise(function(resolve, reject) { killed.on('error', reject); killed.on('spawn', function() { killed.kill('SIGINT'); }); killed.on('close', function(code, signal) { resolve([code, signal, killed.killed]); }); }); var missing = cp.spawn('/thaw/missing-async', []), missingResult = new Promise(function(resolve) { var code; missing.on('error', function(error) { code = error.code; }); missing.on('close', function(exitCode, signal) { resolve([code, exitCode, signal]); }); }); return [events, stdout.join(''), stderr.join(''), child.exitCode, child.signalCode, child.stdin.writableEnded, child instanceof cp.ChildProcess, executed, failed, await killedResult, await missingResult]; };"#,
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_child_process_async_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildProcessAsync = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseChildProcessAsync").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["spawn:true","exit:4:null","close:4:null"],"out:hello","err:hello",4,null,true,true,[null,"callback",""],[6,"failure","","failure"],[null,"SIGINT",true],["ENOENT",null,null]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn child_process_spawn_honors_stdio_timeout_abort_and_detached_options() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_child_process_options");
    fs::write(
            dir.join("index.js"),
            r#"var cp = require('node:child_process'); module.exports = async function () { function close(child) { return new Promise(function(resolve) { child.on('error', function() {}); child.on('close', function(code, signal) { resolve([code, signal]); }); }); } var ignored = cp.spawn('/bin/sh', ['-c', 'printf hidden; printf secret >&2'], { stdio: 'ignore' }); var ignoredResult = await close(ignored); var inheritedOut = '', inheritedErr = '', oldOut = process.stdout, oldErr = process.stderr; process.stdout = { write: function(value) { inheritedOut += Buffer.from(value).toString(); return true; } }; process.stderr = { write: function(value) { inheritedErr += Buffer.from(value).toString(); return true; } }; var inherited = cp.spawn('/bin/sh', ['-c', 'printf visible; printf warning >&2'], { stdio: ['ignore', 'inherit', 'inherit'] }); var inheritedResult = await close(inherited); process.stdout = oldOut; process.stderr = oldErr; var timed = cp.spawn('/bin/sh', ['-c', 'sleep 10'], { timeout: 20 }); var timedResult = await close(timed); var controller = new AbortController(), aborted = cp.spawn('/bin/sh', ['-c', 'sleep 10'], { signal: controller.signal }), abortError; aborted.on('error', function(error) { abortError = [error.name, error.code]; }); var abortedResult = close(aborted); controller.abort('stop'); abortedResult = await abortedResult; var detached = cp.spawn('/bin/sh', ['-c', 'ps -o sid= -p $$'], { detached: true }), detachedOutput = ''; detached.stdout.on('data', function(value) { detachedOutput += value.toString(); }); var detachedResult = await close(detached); var invalid; try { cp.spawn('/bin/true', [], { stdio: 'invalid' }); } catch (error) { invalid = error.code; } return [[ignored.stdin, ignored.stdout, ignored.stderr, ignoredResult], [inherited.stdin, inherited.stdout, inherited.stderr, inheritedOut, inheritedErr, inheritedResult], [timed._timedOut, timedResult], [abortError, abortedResult], [detached.detached, Number(detachedOutput.trim()) === detached.pid, detachedResult], invalid]; };"#,
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_child_process_options_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildProcessOptions = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseChildProcessOptions").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[null,null,null,[0,null]],[null,null,null,"visible","warning",[0,null]],[true,[null,"SIGTERM"]],[["AbortError","ABORT_ERR"],[null,"SIGTERM"]],[true,true,[0,null]],"ERR_INVALID_ARG_VALUE"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn child_process_fork_exchanges_ipc_messages_and_disconnects() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_child_process_fork");
    fs::write(
            dir.join("child.js"),
            "console.log('child-ready'); process.on('message', function(value) { process.send({ answer: value.base + 2, argument: process.argv[2] }, function() { process.disconnect(); }); });",
        )
        .unwrap();
    fs::write(
            dir.join("index.js"),
            r#"var cp = require('node:child_process'); module.exports = async function (childPath) { var child = cp.fork(childPath, ['argument'], { silent: true, execArgv: ['--no-warnings'], execPath: 'node' }), events = [], output = '', callbackCode; child.stdout.on('data', function(value) { output += value.toString(); }); var completed = new Promise(function(resolve, reject) { child.on('error', reject); child.on('spawn', function() { events.push('spawn'); child.send({ base: 40 }, function(error) { callbackCode = error && error.code || null; }); }); child.on('message', function(value) { events.push('message:' + value.answer + ':' + value.argument); }); child.on('disconnect', function() { events.push('disconnect'); }); child.on('close', function(code, signal) { events.push('close:' + code + ':' + signal); resolve(); }); }); await completed; var closed; try { child.send({ late: true }); } catch (error) { closed = error.code; } return [events, output.trim(), callbackCode, child.connected, child.channel, closed, child instanceof cp.ChildProcess]; };"#,
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_child_process_fork_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseChildProcessFork = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseChildProcessFork").unwrap();
    let child_path = dir.join("child.js").to_string_lossy().into_owned();
    let arguments = CString::new(serde_json::to_string(&[child_path]).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["spawn","message:42:argument","disconnect","close:0:null"],"child-ready",null,false,null,"ERR_IPC_CHANNEL_CLOSED",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

/// `IncomingMessage` had `httpVersion` (a string, `"1.1"`) but never
/// `httpVersionMajor`/`httpVersionMinor` (the numeric pair real Node
/// also exposes) -- real trigger: morgan's own `:http-version` token
/// (`req.httpVersionMajor + '.' + req.httpVersionMinor`), which logged
/// `HTTP/undefined.undefined` for every request instead of `HTTP/1.1`.
/// Also missing on the client-response side (`parseResponse`), fixed the
/// same way for consistency even though nothing this session's audits
/// hit it there yet.
#[test]
fn http_server_parses_and_replies_to_a_real_tcp_client() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
                .write_all(b"POST /submit HTTP/1.1\r\nHost: localhost\r\nX-Client: rust\r\nContent-Length: 4\r\nConnection: close\r\n\r\nping")
                .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });

    let dir = temp_registry("builtin_http_server");
    fs::write(
            dir.join("index.js"),
            "var http = require('node:http'); module.exports = async function (port) { var observed = []; var server = http.createServer(async function(request, response) { var body = []; observed.push(request.method, request.url, request.headers['x-client'], request.httpVersion, request.httpVersionMajor, request.httpVersionMinor); await Promise.resolve(); request.on('data', function(chunk) { body.push(chunk.toString()); }); request.on('end', function() { observed.push(body.join('')); response.statusCode = 201; response.statusMessage = 'Stored'; response.setHeader('X-Server', 'thaw'); response.setHeader('Set-Cookie', ['a=1', 'b=2']); response.write('po'); response.end('ng', function() { server.close(); }); }); }); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return [observed, server.listening, server instanceof http.Server]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http_server_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 5);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpServer = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpServer").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["POST","/submit","rust","1.1",1,1,"ping"],false,true]"#
    );
    let response = client.join().unwrap();
    assert!(response.starts_with("HTTP/1.1 201 Stored\r\n"));
    assert!(response.contains("X-Server: thaw\r\n"));
    assert!(response.contains("Set-Cookie: a=1\r\nSet-Cookie: b=2\r\n"));
    assert!(response.contains("Transfer-Encoding: chunked\r\n"));
    assert!(response.ends_with("\r\n\r\n2\r\npo\r\n2\r\nng\r\n0\r\n\r\n"));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

/// `IncomingMessage` never had a `.pipe()` method -- only the hand-rolled
/// `on`/`pause`/`resume` trio built around a buffer-then-replay
/// `_pendingBody`. Real multipart-parsing packages (busboy, and by
/// extension multer) default to exactly `req.pipe(busboy)`
/// (`defaultStreamHandler` in multer's own `make-middleware.js`), so any
/// package built this way failed outright the moment a real request
/// reached it -- `req.pipe is not a function`. Fixed by adding `.pipe()`
/// as a thin shim over the existing `on('data')`/`on('end')` mechanism
/// (mirroring `Readable.prototype.pipe`'s basic contract), rather than
/// rebasing `IncomingMessage` onto a real `Readable` -- the existing
/// buffer-then-replay design already delivers the whole body as one
/// synchronous 'data' event before 'end', which is all `.pipe()` needs to
/// forward correctly.
#[test]
fn http_incoming_message_pipe_forwards_the_buffered_body_and_ends() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"POST /upload HTTP/1.1\r\nHost: localhost\r\nContent-Length: 11\r\nConnection: close\r\n\r\nhello world")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });

    let dir = temp_registry("builtin_http_incoming_message_pipe");
    fs::write(
        dir.join("index.js"),
        "var http = require('node:http'); module.exports = async function (port) { var server = http.createServer(function(request, response) { var received = [], ended = false, destination = { write: function(chunk) { received.push(chunk.toString()); return true; }, end: function() { ended = true; response.end(received.join('') + '|' + ended); server.close(); }, emit: function() {} }; request.pipe(destination); }); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return true; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_incoming_message_pipe_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpIncomingMessagePipe = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpIncomingMessagePipe").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "true");
    let response = client.join().unwrap();
    assert!(response.ends_with("hello world|true"), "{response}");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

/// IncomingMessage unpipe must remove the exact pipe listeners while preserving
/// other destinations, including when called from a data listener snapshot.
#[test]
fn http_incoming_message_unpipe_detaches_destination_listeners() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_incoming_message_unpipe");
    fs::write(
        dir.join("index.js"),
        r#"var http = require('node:http'); module.exports = function() {
  function destination(name) { return { writes: [], events: [], ends: 0,
    write: function(chunk) { this.writes.push(chunk.toString()); return true; },
    end: function() { this.ends++; },
    emit: function(event) { this.events.push(event); }
  }; }
  var before = new http.IncomingMessage(null), first = destination('first'), second = destination('second');
  before.pipe(first); before.pipe(second, { end: false });
  before._queueBody(Buffer.from('A'));
  var returned = before.unpipe(first) === before;
  before._queueBody(Buffer.from('B')); before._finishBody();

  var during = new http.IncomingMessage(null), left = destination('left'), right = destination('right');
  left.write = function(chunk) { this.writes.push(chunk.toString()); during.unpipe(right); return true; };
  during.pipe(left); during.pipe(right);
  during._queueBody(Buffer.from('C')); during._queueBody(Buffer.from('D')); during._finishBody();

  var all = new http.IncomingMessage(null), x = destination('x'), y = destination('y'), removal = new Error('remove marker'), caught = false, nested = false;
  x.emit = function(event) { this.events.push(event); if (event === 'unpipe') { try { all.unpipe(); } catch (error) { nested = error === removal; } throw new Error('notify marker'); } };
  all.pipe(x); all.pipe(y);
  all.on('removeListener', function(event) { if (event === 'data') throw removal; });
  try { all.unpipe(); } catch (error) { caught = error === removal; }
  all._queueBody(Buffer.from('E')); all._finishBody();

  return Promise.resolve().then(function() { return [
    returned, [first.writes, first.ends, first.events], [second.writes, second.ends, second.events],
    [left.writes, left.ends, left.events], [right.writes, right.ends, right.events],
    [caught, nested, all._pipeRecords.length, all.listenerCount('data'), all.listenerCount('end'), x.writes, x.ends, x.events, y.writes, y.ends, y.events]
  ]; });
};"#,
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_incoming_message_unpipe_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let source = CString::new(format!(
        "globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpIncomingMessageUnpipe = module.exports;"
    ))
    .unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpIncomingMessageUnpipe").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,[[],0,["pipe","unpipe"]],[["A","B"],0,["pipe"]],[["C","D"],1,["pipe"]],[[],0,["pipe","unpipe"]],[true,true,0,0,0,[],0,["pipe","unpipe"],[],0,["pipe","unpipe"]]]"#);
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(empty_node_modules);
}

/// Real Node's own `ServerResponse.write()`/`end()` call `this.writeHead
/// (...)` internally if headers haven't been sent yet
/// (`_implicitHeader()`); thaw's own `_head()` built the response head
/// directly and never called `this.writeHead(...)` at all. This broke
/// any middleware using the extremely common `on-headers` package to
/// hook "right before headers are sent" -- real trigger: morgan's own
/// `response-time` token, which monkey-patches `res.writeHead` to
/// timestamp `res._startAt`; since `res.writeHead` was never invoked
/// internally, the hook never fired and `response-time` always printed
/// `-` instead of a real number (confirmed via reading `on-headers`' and
/// morgan's own real source, and independently confirming
/// `process.hrtime()` itself works fine in isolation). Fixed by having
/// `_head()` call `this.writeHead(this.statusCode)` once, gated on
/// `!this.headersSent` (mirroring Node's own `_implicitHeader` timing);
/// a monkey-patched `writeHead` (this test's own stand-in for
/// `on-headers`) now fires exactly once even across a real end-to-end
/// request, and an *explicit* `res.writeHead(...)` call from user code
/// still wins (its own status/headers aren't clobbered by a second,
/// implicit call) -- covered already by every other test in this file
/// that calls `response.setHeader`/sets `statusCode` before `.end()`,
/// still green with this fix in place.
#[test]
fn server_response_calls_write_head_once_if_not_already_called() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        stream
            .write_all(b"GET /hello HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });

    let dir = temp_registry("builtin_http_implicit_write_head");
    fs::write(
        dir.join("index.js"),
        "var http = require('node:http'); module.exports = async function (port) { var calls = 0; var server = http.createServer(function(request, response) { var real = response.writeHead; response.writeHead = function(statusCode, headers) { calls++; return real.call(response, statusCode, headers); }; response.end('ok', function() { server.close(); }); }); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return calls; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_implicit_write_head_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseImplicitWriteHead = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseImplicitWriteHead").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "1");
    let response = client.join().unwrap();
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"), "{response}");
    assert!(response.ends_with("ok"), "{response}");
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(empty_node_modules);
}

#[test]
fn http_server_supports_standard_timeout_configuration() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_server_timeout");
    fs::write(
        dir.join("index.js"),
        "var http = require('node:http'); module.exports = function () { var called = false, server = http.createServer(); var result = server.setTimeout(1234, function() { called = true; }); server.emit('timeout'); return [result === server, server.timeout, called]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_server_timeout_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpServerTimeout = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpServerTimeout").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result = unsafe {
        CStr::from_ptr(thaw_quickjs::thaw_js_call(
            function.as_ptr(),
            arguments.as_ptr(),
        ))
    }
    .to_string_lossy();
    assert_eq!(result, "[true,1234,true]");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_server_handles_chunked_bodies_and_pipelined_keep_alive_requests() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        let read_response = |stream: &mut TcpStream| {
            let mut response = Vec::new();
            while !response.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                response.push(byte[0]);
            }
            let headers = String::from_utf8(response).unwrap();
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.parse::<usize>().ok())
                })
                .unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            (headers, body)
        };
        stream.write_all(
            b"POST /first HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n4\r\nthaw\r\n3\r\n-ok\r\n0\r\n\r\nGET /second HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        ).unwrap();
        let first = read_response(&mut stream);
        let second = read_response(&mut stream);
        (first, second)
    });

    let dir = temp_registry("builtin_http_server_keep_alive");
    fs::write(
        dir.join("index.js"),
        "var http = require('node:http'); module.exports = async function(port) { var count = 0, server = http.createServer(function(request, response) { count++; if (request.method === 'POST') { var chunks = []; request.on('data', function(chunk) { chunks.push(chunk); }); request.on('end', function() { response.end(Buffer.concat(chunks)); }); } else response.end(String(count), function() { server.close(); }); }); setTimeout(function() { if (server.listening) server.close(); }, 1000); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return count; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_server_keep_alive_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpKeepAlive = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpKeepAlive").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result = unsafe {
        CStr::from_ptr(thaw_quickjs::thaw_js_call(
            function.as_ptr(),
            arguments.as_ptr(),
        ))
    }
    .to_string_lossy();
    assert_eq!(result, "2");
    let ((first_headers, first_body), (second_headers, second_body)) = client.join().unwrap();
    assert!(first_headers
        .to_ascii_lowercase()
        .contains("connection: keep-alive"));
    assert_eq!(first_body, b"thaw-ok");
    assert!(second_headers
        .to_ascii_lowercase()
        .contains("connection: close"));
    assert_eq!(second_body, b"2");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_server_streams_sse_before_response_end() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        stream
            .write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut first = [0; 512];
        let count = stream.read(&mut first).unwrap();
        let first = String::from_utf8_lossy(&first[..count]).into_owned();
        assert!(first.contains("Transfer-Encoding: chunked"));
        assert!(first.contains("data: first"));
        assert!(!first.contains("data: second"));
        let mut rest = String::new();
        stream.read_to_string(&mut rest).unwrap();
        (first, rest)
    });

    let dir = temp_registry("builtin_http_server_sse");
    fs::write(
        dir.join("index.js"),
        "var http = require('node:http'); module.exports = async function(port) { var server = http.createServer(function(request, response) { response.setHeader('Content-Type', 'text/event-stream'); response.write('data: first\\n\\n'); setTimeout(function() { response.end('data: second\\n\\n', function() { server.close(); }); }, 200); }); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return true; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_server_sse_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpSse = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpSse").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result = unsafe {
        CStr::from_ptr(thaw_quickjs::thaw_js_call(
            function.as_ptr(),
            arguments.as_ptr(),
        ))
    }
    .to_string_lossy();
    assert_eq!(result, "true");
    let (_, rest) = client.join().unwrap();
    assert!(rest.contains("data: second"));
    assert!(rest.ends_with("0\r\n\r\n"));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_server_emits_websocket_upgrade_with_head_bytes() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        stream
            .write_all(b"GET /socket HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive, Upgrade\r\nUpgrade: websocket\r\n\r\nhead")
            .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });

    let dir = temp_registry("builtin_http_server_upgrade");
    fs::write(
        dir.join("index.js"),
        "var http = require('node:http'); module.exports = async function(port) { var observed, requests = 0, server = http.createServer(); server.on('request', function() { requests++; }); server.on('upgrade', function(request, socket, head) { observed = [request.method, request.url, request.headers.upgrade, head.toString(), request.complete]; socket.end('HTTP/1.1 101 Switching Protocols\\r\\nConnection: Upgrade\\r\\nUpgrade: websocket\\r\\n\\r\\n', function() { server.close(); }); }); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return observed.concat(requests); };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_http_server_upgrade_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpUpgrade = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpUpgrade").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result = unsafe {
        CStr::from_ptr(thaw_quickjs::thaw_js_call(
            function.as_ptr(),
            arguments.as_ptr(),
        ))
    }
    .to_string_lossy();
    assert_eq!(result, r#"["GET","/socket","websocket","head",true,0]"#);
    assert!(client
        .join()
        .unwrap()
        .starts_with("HTTP/1.1 101 Switching Protocols\r\n"));
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn https_client_verifies_a_custom_ca_and_parses_http() {
    use rustls::pki_types::ServerName;
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    use rustls::{
        ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
    };
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Command;
    use std::sync::Arc;

    let certificate_dir = temp_registry("builtin_https_certificate");
    let key_pem = certificate_dir.join("key.pem");
    let cert_pem = certificate_dir.join("cert.pem");
    let key_der = certificate_dir.join("key.der");
    let cert_der = certificate_dir.join("cert.der");
    assert!(Command::new("openssl")
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=localhost",
            "-addext",
            "subjectAltName=DNS:localhost,IP:127.0.0.1",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature,keyEncipherment",
            "-addext",
            "extendedKeyUsage=serverAuth",
            "-keyout",
        ])
        .arg(&key_pem)
        .arg("-out")
        .arg(&cert_pem)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args(["x509", "-in"])
        .arg(&cert_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&cert_der)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("openssl")
        .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
        .arg(&key_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&key_der)
        .status()
        .unwrap()
        .success());
    let certificate = CertificateDer::from(fs::read(&cert_der).unwrap());
    let private_key = PrivatePkcs8KeyDer::from(fs::read(&key_der).unwrap()).into();
    let config = Arc::new(
        ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(vec![certificate], private_key)
            .unwrap(),
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_config = config.clone();
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let connection = ServerConnection::new(server_config).unwrap();
        let mut stream = StreamOwned::new(connection, socket);
        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let read = stream.read(&mut chunk).unwrap();
            if read == 0 {
                break;
            }
            request.extend_from_slice(&chunk[..read]);
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request.starts_with("GET /secure HTTP/1.1\r\n"));
        stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nX-Secure: yes\r\nConnection: close\r\n\r\nsecret")
                .unwrap();
        stream.conn.send_close_notify();
        stream.flush().unwrap();
    });

    let dir = temp_registry("builtin_https_client");
    fs::write(
            dir.join("index.js"),
            "var https = require('node:https'); module.exports = async function (port, ca) { return await new Promise(function(resolve, reject) { var request = https.get({ hostname: '127.0.0.1', port: port, path: '/secure', ca: ca }, function(response) { var body = []; response.setEncoding('utf8'); response.on('data', function(chunk) { body.push(chunk); }); response.on('end', function() { resolve([response.statusCode, response.headers['x-secure'], body.join(''), request.protocol, request.socket.encrypted, request.socket.authorized, https.globalAgent.protocol]); }); }); request.on('error', reject); }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_https_client_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 7);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpsClient = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpsClient").unwrap();
    let ca = fs::read_to_string(&cert_pem).unwrap();
    let arguments = CString::new(serde_json::to_string(&(port, &ca)).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[200,"yes","secret","https:",true,true,"https:"]"#
    );
    server.join().unwrap();

    // A direct TLS client must receive data while its write half is still open.
    let greeting_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let greeting_port = greeting_listener.local_addr().unwrap().port();
    let greeting_server = std::thread::spawn(move || {
        let (socket, _) = greeting_listener.accept().unwrap();
        socket.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
        let mut stream = StreamOwned::new(ServerConnection::new(config).unwrap(), socket);
        stream.write_all(b"greeting").unwrap();
        stream.flush().unwrap();
        let mut received = [0_u8; 1];
        assert_eq!(stream.read(&mut received).unwrap(), 0);
        stream.conn.send_close_notify();
        stream.flush().unwrap();
    });
    let direct_dir = temp_registry("builtin_tls_read_before_end");
    fs::write(
        direct_dir.join("index.js"),
        "var tls = require('node:tls'); module.exports = function(port, ca) { return new Promise(function(resolve, reject) { var chunks = [], pausedSnapshot, writableOnData = false, endCallbacks = 0, ends = 0, closes = 0; var socket = tls.connect({ host: '127.0.0.1', port: port, ca: ca }); socket.on('error', reject); socket.on('secureConnect', function() { socket.pause(); setTimeout(function() { pausedSnapshot = chunks.length; socket.resume(); }, 20); }); socket.on('data', function(chunk) { writableOnData = socket.writable; chunks.push(chunk.toString()); if (chunks.join('') === 'greeting') socket.end(function() { endCallbacks++; }); }); socket.on('end', function() { ends++; }); socket.on('close', function() { closes++; resolve([pausedSnapshot, chunks.join(''), writableOnData, endCallbacks, ends, closes]); }); }); };",
    )
    .unwrap();
    let direct_node_modules = temp_registry("builtin_tls_read_before_end_node_modules");
    let (direct_bundle, _, direct_file_count, _) =
        bundle_commonjs_package(&direct_node_modules, "tls-pkg", &direct_dir, "index.js")
            .unwrap();
    assert_eq!(direct_file_count, 2);
    let direct_script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; {direct_bundle} globalThis.exerciseTlsReadBeforeEnd = module.exports;");
    let direct_source = CString::new(direct_script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(direct_source.as_ptr()), 1);
    let direct_function = CString::new("exerciseTlsReadBeforeEnd").unwrap();
    let direct_arguments = CString::new(serde_json::to_string(&(greeting_port, &ca)).unwrap()).unwrap();
    let direct_result = thaw_quickjs::thaw_js_call(direct_function.as_ptr(), direct_arguments.as_ptr());
    let direct_result = unsafe { CStr::from_ptr(direct_result) }.to_string_lossy();
    assert_eq!(direct_result, r#"[0,"greeting",true,1,1,1]"#);
    greeting_server.join().unwrap();
    let _ = fs::remove_dir_all(&direct_dir);
    let _ = fs::remove_dir_all(&direct_node_modules);

    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let server_port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(fs::read(&cert_der).unwrap()))
        .unwrap();
    let client_config = Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    );
    let tls_client = std::thread::spawn(move || {
        let mut responses = Vec::new();
        let mut first_stream = None;
        for close_first in [false, true] {
            let socket = loop {
                match TcpStream::connect(("127.0.0.1", server_port)) {
                    Ok(socket) => break socket,
                    Err(_) => std::thread::sleep(std::time::Duration::from_millis(2)),
                }
            };
            socket.set_read_timeout(Some(std::time::Duration::from_secs(3))).unwrap();
            let connection = ClientConnection::new(
                client_config.clone(),
                ServerName::try_from("localhost").unwrap(),
            )
            .unwrap();
            let mut stream = StreamOwned::new(connection, socket);
            stream
                .write_all(b"GET /from-rust HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .unwrap();
            if close_first {
                stream.conn.send_close_notify();
            }
            stream.flush().unwrap();
            // First request needs a response before EOF; second ends first and
            // needs the accepted socket's write half for an async response.
            let mut first = [0_u8; 1];
            stream.read_exact(&mut first).unwrap();
            let mut response = String::from_utf8(first.to_vec()).unwrap();
            let mut chunk = [0_u8; 1024];
            while !response.ends_with("\r\n\r\nsecure-server") {
                let read = stream.read(&mut chunk).unwrap();
                assert!(read > 0);
                response.push_str(std::str::from_utf8(&chunk[..read]).unwrap());
            }
            if !close_first {
                first_stream = Some(stream); // Keep the peer's write half open through response.finish.
            }
            responses.push(response);
        }
        drop(first_stream);
        responses
    });
    let server_dir = temp_registry("builtin_https_server");
    fs::write(
            server_dir.join("index.js"),
            "var https = require('node:https'); module.exports = async function (port, cert, key) { var observed = [], terminal = [], completed = 0, firstFinished = false, firstSocket; var server = https.createServer({ cert: cert, key: key }, function(request, response) { observed.push([request.method, request.url, request.socket.encrypted, request.socket.authorized]); var first = observed.length === 1; if (first) firstSocket = request.socket; response.statusCode = 202; response.setHeader('X-TLS', 'yes'); if (!first) response.setHeader('X-First-Finished', firstFinished ? 'yes' : 'no'); setTimeout(function() { response.end('secure-server', function() { if (first) firstFinished = true; if (++completed === 2) { firstSocket.destroy(); server.close(); } }); }, 10); }); server.on('secureConnection', function(socket) { var events = { end: 0, close: 0 }; terminal.push(events); socket.pause(); setTimeout(function() { socket.resume(); }, 5); socket.on('end', function() { events.end++; }); socket.on('close', function() { events.close++; }); }); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return [observed, server.listening, server instanceof https.Server, terminal.map(function(events) { return events.end <= 1 && events.close === 1; }), terminal[1].end]; };",
        )
        .unwrap();
    let server_node_modules = temp_registry("builtin_https_server_node_modules");
    let (server_bundle, _, server_file_count, _) =
        bundle_commonjs_package(&server_node_modules, "secure-pkg", &server_dir, "index.js")
            .unwrap();
    assert_eq!(server_file_count, 7);
    let server_script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; {server_bundle} globalThis.exerciseHttpsServer = module.exports;");
    let server_source = CString::new(server_script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(server_source.as_ptr()), 1);
    let server_function = CString::new("exerciseHttpsServer").unwrap();
    let server_arguments = CString::new(
        serde_json::to_string(&(
            server_port,
            fs::read_to_string(&cert_pem).unwrap(),
            fs::read_to_string(&key_pem).unwrap(),
        ))
        .unwrap(),
    )
    .unwrap();
    let server_result =
        thaw_quickjs::thaw_js_call(server_function.as_ptr(), server_arguments.as_ptr());
    let server_result = unsafe { CStr::from_ptr(server_result) }.to_string_lossy();
    assert_eq!(
        server_result,
        r#"[[["GET","/from-rust",true,true],["GET","/from-rust",true,true]],false,true,[true,true],1]"#
    );
    let responses = tls_client.join().unwrap();
    assert!(responses[1].contains("X-First-Finished: yes\r\n"));
    for response in responses {
        assert!(response.starts_with("HTTP/1.1 202 Accepted\r\n"));
        assert!(response.contains("X-TLS: yes\r\n"));
        assert!(response.ends_with("\r\n\r\nsecure-server"));
    }
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    let _ = fs::remove_dir_all(&server_dir);
    let _ = fs::remove_dir_all(&server_node_modules);
    let _ = fs::remove_dir_all(&certificate_dir);
}

#[test]
fn net_server_accepts_and_replies_to_a_real_tcp_client() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::time::Duration;

    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let client = std::thread::spawn(move || {
        (0..3)
            .map(|index| {
                let mut stream = loop {
                    match TcpStream::connect(("127.0.0.1", port)) {
                        Ok(stream) => break stream,
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                };
                stream.write_all(format!("ping{index}").as_bytes()).unwrap();
                stream.shutdown(Shutdown::Write).unwrap();
                let mut response = String::new();
                stream.read_to_string(&mut response).unwrap();
                response
            })
            .collect::<Vec<_>>()
    });

    let dir = temp_registry("builtin_net_server");
    let source = "var net = require('node:net'); module.exports = async function (port) { var events = [], handled = 0; var server; var closed = new Promise(function(resolve, reject) { server = net.createServer(function(socket) { events.push('connection:' + socket.remoteFamily); socket.on('error', reject); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); handled++; socket.end('pong' + handled, function() { if (handled === 3) server.close(); }); }); }); server.on('error', reject); server.on('listening', function() { var address = server.address(); events.push('listening:' + address.address + ':' + address.port); }); server.on('close', function() { events.push('close'); resolve(); }); server.listen(port, '127.0.0.1'); }); await closed; return [events, server.listening, server.address().port, server.connections, server.ref() === server, server.unref() === server]; };";
    fs::write(
        dir.join("index.js"),
        source.replace("server.listen(port, '127.0.0.1');", "server.listen(port, function() {});"),
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_net_server_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetServer = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNetServer").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        format!(
            r#"[["listening:127.0.0.1:{port}","connection:IPv4","data:ping0","connection:IPv4","data:ping1","connection:IPv4","data:ping2","close"],false,{port},0,true,true]"#
        )
    );
    assert_eq!(client.join().unwrap(), ["pong1", "pong2", "pong3"]);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn unreferenced_net_server_does_not_hold_the_event_loop_open() {
    use std::ffi::{CStr, CString};
    use std::net::TcpListener;

    let probe = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let dir = temp_registry("builtin_unref_net_server");
    fs::write(
        dir.join("index.js"),
        "var net = require('node:net'), server; module.exports = { start: function(port) { server = net.createServer(); server.listen(port); server.unref(); return server.listening; }, close: function() { server.close(); } };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_unref_net_server_node_modules");
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = CString::new(format!(
        "globalThis.module = {{ exports: {{}} }}; globalThis.exports = module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.startUnrefServer = module.exports.start; globalThis.closeUnrefServer = module.exports.close;"
    ))
    .unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(script.as_ptr()), 1);
    let start = CString::new("startUnrefServer").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result = thaw_quickjs::thaw_js_call(start.as_ptr(), arguments.as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "true");
    thaw_quickjs::thaw_js_run_event_loop();
    let close = CString::new("closeUnrefServer").unwrap();
    let arguments = CString::new("[]").unwrap();
    thaw_quickjs::thaw_js_call(close.as_ptr(), arguments.as_ptr());
    let _ = fs::remove_dir_all(dir);
    let _ = fs::remove_dir_all(empty_node_modules);
}

/// End-to-end confirmation that `dns.lookup`'s new native resolution
/// path (`dns_lookup_json_resolves_localhost_via_the_real_os_resolver`
/// in thaw-quickjs covers the mechanism hermetically) is actually wired
/// through the JS `node:dns` builtin correctly: a real hostname other
/// than `"localhost"` (which has its own hardcoded fast path,
/// unaffected either way) now resolves to real addresses instead of
/// always failing with `ENOTFOUND`. Gated behind the same real-network
/// opt-in this project's other live-network tests already use, so the
/// default hermetic suite never depends on external DNS.
#[test]
fn node_dns_lookup_resolves_a_real_hostname_when_network_integration_is_enabled() {
    use std::ffi::{CStr, CString};
    if std::env::var("THAW_RUN_NPM_INTEGRATION").as_deref() != Ok("1") {
        return;
    }
    let dir = temp_registry("builtin_dns_real_lookup");
    fs::write(
        dir.join("index.js"),
        "var dns = require('node:dns'); module.exports = function () {\n\
             return new Promise(function (resolve) {\n\
                 dns.lookup('dns.google', { all: true }, function (err, addresses) {\n\
                     if (err) { resolve(['error', err.code]); return; }\n\
                     resolve(['ok', addresses.length > 0, addresses.every(function (entry) {\n\
                         return (entry.family === 4 || entry.family === 6) && typeof entry.address === 'string';\n\
                     })]);\n\
                 });\n\
             });\n\
         };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_dns_real_lookup_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDnsLookup = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseDnsLookup").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"["ok",true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_server_emits_headers_before_body_and_keeps_early_response_body_framed() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        stream.write_all(b"POST /one HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\nConnection: keep-alive\r\n\r\n").unwrap();
        let mut received = Vec::new();
        let mut scratch = [0; 512];
        while !received.windows(5).any(|part| part == b"ready") {
            match stream.read(&mut scratch) {
                Ok(0) | Err(_) => break,
                Ok(count) => received.extend_from_slice(&scratch[..count]),
            }
        }
        let saw_ready = received.windows(5).any(|part| part == b"ready");
        stream.write_all(b"bo").unwrap();
        stream.write_all(b"dyGET /two HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        loop { match stream.read(&mut scratch) {
            Ok(0) | Err(_) => break,
            Ok(count) => received.extend_from_slice(&scratch[..count]),
        } }
        (saw_ready, String::from_utf8_lossy(&received).into_owned())
    });

    let dir = temp_registry("builtin_http_server_streaming_pipeline");
    fs::write(dir.join("index.js"), r#"var http = require('node:http'); module.exports = function(port) { return new Promise(function(resolve, reject) {
      var server = http.createServer(), first = { headersComplete: null, data: '', ends: 0, endComplete: null }, second = [], sockets = [];
      var timeout = setTimeout(function() { sockets.forEach(function(socket) { socket.destroy(); }); server.close(); }, 2000);
      server.on('connection', function(socket) { sockets.push(socket); });
      server.on('request', function(req, res) {
        if (req.url === '/one') {
          first.headersComplete = req.complete;
          req.on('data', function(chunk) { first.data += chunk.toString(); });
          req.on('end', function() { first.ends++; first.endComplete = req.complete; });
          res.write('ready'); res.end('first');
        } else {
          second.push([req.method, req.url, req.complete]);
          res.end('second', function() { server.close(); });
        }
      });
      server.on('error', reject);
      server.on('close', function() { clearTimeout(timeout); resolve([first.headersComplete, first.data, first.ends, first.endComplete, second]); });
      server.listen(port, '127.0.0.1');
    }); };"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_server_streaming_pipeline_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpServerStreamingPipeline = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpServerStreamingPipeline").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[false,"body",1,true,[["GET","/two",true]]]"#);
    let (saw_ready, wire) = client.join().unwrap();
    assert!(saw_ready, "handler did not respond before body: {wire}");
    assert!(wire.contains("first") && wire.contains("second"), "{wire}");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_server_streams_chunked_body_and_rejects_short_content_length() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::time::Duration;

    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.write_all(b"POST /chunks HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\nConnection: keep-alive\r\n\r\n").unwrap();
        for part in [&b"2\r"[..], &b"\nAB\r\n1\r\nC\r\n0\r\nX-Trail: yes\r"[..], &b"\n\r\n"[..]] {
            stream.write_all(part).unwrap();
            std::thread::sleep(Duration::from_millis(2));
        }
        stream.write_all(b"POST /short HTTP/1.1\r\nHost: localhost\r\nContent-Length: 3\r\nConnection: close\r\n\r\nx").unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut received = String::new();
        let _ = stream.read_to_string(&mut received);
        received
    });

    let dir = temp_registry("builtin_http_server_streaming_chunked");
    fs::write(dir.join("index.js"), r#"var http = require('node:http'); module.exports = function(port) { return new Promise(function(resolve, reject) {
      var server = http.createServer(), chunks = [], short = [], faults = 0, before = [], after = [], sockets = [];
      var timeout = setTimeout(function() { sockets.forEach(function(socket) { socket.destroy(); }); server.close(); }, 2000);
      server.on('connection', function(socket) { sockets.push(socket); });
      server.on('request', function(req, res) {
        if (req.url === '/chunks') {
          before.push(req.complete);
          req.on('data', function(chunk) { chunks.push(chunk.toString()); });
          req.on('end', function() { after.push(req.complete); res.end('ok'); });
        } else {
          short.push(req.complete);
          req.on('end', function() { short.push('unexpected-end'); });
        }
      });
      server.on('clientError', function(error, socket) { faults++; socket.destroy(); server.close(); });
      server.on('error', reject);
      server.on('close', function() { clearTimeout(timeout); resolve([before, chunks.join(''), after, short, faults]); });
      server.listen(port, '127.0.0.1');
    }); };"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_server_streaming_chunked_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpServerStreamingChunked = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpServerStreamingChunked").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[[false],"ABC",[true],[false],1]"#);
    let _wire = client.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_server_short_body_client_error_throw_still_aborts_and_destroys_socket() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_server_streaming_error_cleanup");
    fs::write(dir.join("index.js"), r#"var http = require('node:http'), EventEmitter = require('node:events'); module.exports = function() {
      var accept, network = new EventEmitter(), socket = new EventEmitter(), marker = new Error('client error listener'), request, ended = 0, destroyed = false, caught = false;
      network.createServer = function(options, callback) { accept = callback; return network; };
      socket.destroy = function() { destroyed = true; socket.destroyed = true; };
      var server = http.createServer({ _transport: network });
      server.on('request', function(value) { request = value; request.on('end', function() { ended++; }); });
      server.on('clientError', function() { throw marker; });
      accept(socket);
      socket.emit('data', Buffer.from('POST /short HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\n\r\nx'));
      try { socket.emit('end'); } catch (error) { caught = error === marker; }
      return [caught, destroyed, request.aborted, request.complete, request._bodyQueue.length, ended];
    };"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_server_streaming_error_cleanup_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpServerStreamingErrorCleanup = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpServerStreamingErrorCleanup").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "[true,true,true,false,0,0]");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_server_reuse_callback_stays_bound_to_its_own_request() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_server_reuse_record");
    fs::write(dir.join("index.js"), r#"var http = require('node:http'), EventEmitter = require('node:events'); module.exports = function() {
      var accept, network = new EventEmitter(), socket = new EventEmitter(), first, second, seen = [];
      network.createServer = function(options, callback) { accept = callback; return network; };
      socket.write = function() { return true; };
      socket.destroy = function() { socket.destroyed = true; };
      var server = http.createServer({ _transport: network });
      server.on('request', function(request, response) {
        seen.push(request.url);
        if (request.url === '/one') { first = response; response.end('one'); }
        else if (request.url === '/two') { second = response; first._reuse(); }
      });
      accept(socket);
      socket.emit('data', Buffer.from('GET /one HTTP/1.1\r\nHost: localhost\r\n\r\nGET /two HTTP/1.1\r\nHost: localhost\r\n\r\nGET /three HTTP/1.1\r\nHost: localhost\r\n\r\n'));
      return [seen, Boolean(second), second && second.writableEnded];
    };"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_server_reuse_record_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpServerReuseRecord = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpServerReuseRecord").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[["/one","/two"],true,false]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_response_framing_keeps_pipelined_messages_aligned() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut requests = Vec::new();
        for (method, path) in [("GET", "/explicit"), ("GET", "/empty"), ("HEAD", "/head"), ("GET", "/204"), ("GET", "/204-framed"), ("GET", "/304"), ("GET", "/205-buffered"), ("GET", "/205-streamed"), ("GET", "/205-explicit"), ("GET", "/last")] {
            requests.extend_from_slice(format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes());
        }
        stream.write_all(&requests).unwrap();
        fn read_head(stream: &mut TcpStream) -> String {
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            String::from_utf8(head).unwrap()
        }
        let first = read_head(&mut stream);
        assert!(first.starts_with("HTTP/1.1 200 OK\r\n"), "{first}");
        assert!(first.contains("Transfer-Encoding: chunked\r\n"), "{first}");
        let mut chunk = [0; 11];
        stream.read_exact(&mut chunk).unwrap();
        assert_eq!(&chunk, b"1\r\nx\r\n0\r\n\r\n");
        let second = read_head(&mut stream);
        assert!(second.contains("Transfer-Encoding: chunked\r\n"), "{second}");
        stream.read_exact(&mut chunk).unwrap();
        assert_eq!(&chunk, b"1\r\ny\r\n0\r\n\r\n");
        let head = read_head(&mut stream);
        assert!(head.starts_with("HTTP/1.1 200 OK\r\n"), "{head}");
        assert!(head.contains("Content-Length: 4\r\n"), "{head}");
        let no_content = read_head(&mut stream);
        assert!(no_content.starts_with("HTTP/1.1 204 No Content\r\n"), "{no_content}");
        let invalid_no_content = read_head(&mut stream);
        assert!(invalid_no_content.starts_with("HTTP/1.1 204 No Content\r\n"), "{invalid_no_content}");
        assert!(!invalid_no_content.contains("Content-Length:"), "{invalid_no_content}");
        assert!(!invalid_no_content.contains("Transfer-Encoding:"), "{invalid_no_content}");
        let not_modified = read_head(&mut stream);
        assert!(not_modified.starts_with("HTTP/1.1 304 Not Modified\r\n"), "{not_modified}");
        assert!(not_modified.contains("Transfer-Encoding: chunked\r\n"), "{not_modified}");
        for _ in 0..3 {
            let reset = read_head(&mut stream);
            assert!(reset.starts_with("HTTP/1.1 205 Reset Content\r\n"), "{reset}");
            assert_eq!(reset.matches("Content-Length: 0\r\n").count(), 1, "{reset}");
            assert!(!reset.contains("Transfer-Encoding:"), "{reset}");
        }
        let last = read_head(&mut stream);
        assert!(last.starts_with("HTTP/1.1 200 OK\r\n"), "{last}");
        assert!(last.contains("Content-Length: 2\r\n"), "{last}");
        let mut body = [0; 2];
        stream.read_exact(&mut body).unwrap();
        assert_eq!(&body, b"ok");
    });

    let dir = temp_registry("builtin_http_response_framing");
    fs::write(dir.join("index.js"), r#"var http = require('node:http'); module.exports = async function(port) {
      var callbacks = [], guards = [], server = http.createServer(function(request, response) {
        if (request.url === '/explicit') { response.setHeader('Transfer-Encoding', 'chunked'); response.writeHead(200); response.statusCode = 204; response.end('x'); }
        else if (request.url === '/empty') { response.write('', function() { callbacks.push('empty-head'); }); response.write('', function() { callbacks.push('empty-no-packet'); }); response.write('y', function() { callbacks.push('data'); }); response.end(function() { callbacks.push('end'); }); }
        else if (request.url === '/head') { response.setHeader('Content-Length', '4'); response.writeHead(200); for (var change of [function() { response.setHeader('X-Late', 'x'); }, function() { response.removeHeader('Content-Length'); }, function() { response.writeHead(201); }]) { try { change(); guards.push(false); } catch (error) { guards.push(error.code === 'ERR_HTTP_HEADERS_SENT'); } } response.write('drop'); response.end(); }
        else if (request.url === '/204') { response.statusCode = 204; response.write('bad'); response.end(); }
        else if (request.url === '/204-framed') { response.writeHead(204, { 'Content-Length': '9', 'Transfer-Encoding': 'chunked' }); response.end('bad'); }
        else if (request.url === '/304') { response.statusCode = 304; response.setHeader('Transfer-Encoding', 'chunked'); response.write('bad'); response.end(); }
        else if (request.url === '/205-buffered') { response.statusCode = 205; response.end('bad'); }
        else if (request.url === '/205-streamed') { response.statusCode = 205; response.write('bad'); response.end('more'); }
        else if (request.url === '/205-explicit') { response.writeHead(205, { 'Content-Length': '9', 'Transfer-Encoding': 'chunked' }); response.flushHeaders(); response.end('bad'); }
        else response.end('ok', function() { server.close(); });
      });
      await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); });
      return [callbacks.sort(), guards];
    };"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_response_framing_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpResponseFraming = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpResponseFraming").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[["data","empty-head","empty-no-packet","end"],[true,true,true]]"#);
    client.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_client_205_waits_for_declared_framing_or_eof() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_client_205_framing");
    fs::write(dir.join("index.js"), r#"var http = require('node:http'), EventEmitter = require('node:events');
module.exports = async function() {
  async function receive(mode) {
    var socket = new EventEmitter(), response, data = 0, ends = 0, errors = 0;
    socket.write = function() { return true; };
    socket.destroy = function() { this.emit('close'); };
    var request = http.request('http://localhost/', { agent: false, _transport: { createConnection: function() { return socket; } } }, function(value) {
      response = value;
      response.on('data', function() { data++; });
      response.on('end', function() { ends++; });
    });
    request.on('error', function() { errors++; });
    request.end(); socket.emit('connect');
    var head = 'HTTP/1.1 205 Reset Content\r\n' + (mode === 'length' ? 'Content-Length: 0\r\n' : mode === 'chunked' ? 'Transfer-Encoding: chunked\r\n' : '') + '\r\n';
    socket.emit('data', Buffer.from(head));
    var before = response.complete;
    if (mode === 'chunked') socket.emit('data', Buffer.from('0\r\nX-End: yes\r\n\r\n'));
    socket.emit('end');
    await Promise.resolve();
    return [response.statusCode, before, response.complete, data, ends, response.trailers['x-end'] || '', errors];
  }
  return [await receive('length'), await receive('chunked'), await receive('eof')];
};"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_client_205_framing_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; {bundle} globalThis.exerciseHttpClient205Framing = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpClient205Framing").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[[205,true,true,0,1,"",0],[205,false,true,0,1,"yes",0],[205,false,true,0,1,"",0]]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_client_write_accepts_second_argument_callback() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_client_write_callback");
    fs::write(dir.join("index.js"), "var http = require('node:http'); module.exports = async function() { var calls = 0, request = http.request('http://localhost/'); request.write(Buffer.from('x'), function() { calls++; }); await Promise.resolve(); return calls; };").unwrap();
    let empty_node_modules = temp_registry("builtin_http_client_write_callback_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpClientWriteCallback = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpClientWriteCallback").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "1");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_incoming_pipe_waits_for_all_drains_and_keeps_user_pause() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_pipe_backpressure");
    fs::write(dir.join("index.js"), r#"var http = require('node:http'), EventEmitter = require('node:events');
module.exports = async function() {
  var gate = false, socket = { _setHttpReadPaused: function(value) { gate = value; }, destroy: function() {} };
  var message = new http.IncomingMessage(socket);
  message._bodyReadGate = function() { socket._setHttpReadPaused(message._bodyBytes >= 16384 || message._blockedPipes()); };
  message._bodyResume = function() { message._syncReadGate(); };
  function destination() { var result = new EventEmitter(); result.seen = []; result.write = function(chunk) { this.seen.push(chunk.toString()); return this.seen.length > 1; }; result.end = function() {}; return result; }
  var first = destination(), second = destination();
  message.pipe(first); message.pipe(second);
  message._queueBody(Buffer.from('A')); message._queueBody(Buffer.from('B'));
  await Promise.resolve();
  var initial = first.seen.join('') === 'A' && second.seen.join('') === 'A' && gate && message._bodyBytes === 1;
  first.emit('drain'); await Promise.resolve();
  var oneDrain = first.seen.length === 1 && second.seen.length === 1 && message._blockedPipes();
  message.pause(); second.emit('drain'); await Promise.resolve();
  var userPause = first.seen.length === 1 && second.seen.length === 1 && message.isPaused();
  message.resume(); await Promise.resolve();
  var resumed = first.seen.join('') === 'AB' && second.seen.join('') === 'AB' && !message._blockedPipes();
  message.unpipe(first); message._abortBody();
  var clean = first.listenerCount('drain') === 0 && second.listenerCount('drain') === 0 && message._pipeRecords.length === 0;

  var synchronous = new http.IncomingMessage(null), syncDestination = new EventEmitter(), syncWrites = 0;
  syncDestination.write = function() { syncWrites++; this.emit('drain'); return false; };
  synchronous.pipe(syncDestination); synchronous._queueBody(Buffer.from('x')); await Promise.resolve();
  var syncSafe = syncWrites === 1 && !synchronous._blockedPipes() && syncDestination.listenerCount('drain') === 0;

  var reentrant = new http.IncomingMessage(null), removed = new EventEmitter(), removedWrites = 0, once = false;
  removed.write = function() { removedWrites++; return false; };
  removed.on('newListener', function(name) { if (name === 'drain' && !once) { once = true; reentrant.unpipe(removed); } });
  reentrant.pipe(removed); reentrant._queueBody(Buffer.from('z')); await Promise.resolve();
  var reentrySafe = removedWrites === 0 && removed.listenerCount('drain') === 0 && reentrant._pipeRecords.length === 0;
  return [initial, oneDrain, userPause, resumed, clean, syncSafe, reentrySafe];
};"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_pipe_backpressure_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpPipeBackpressure = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpPipeBackpressure").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "[true,true,true,true,true,true,true]");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_transport_gate_does_not_override_socket_pause_or_duplicate_pumps() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_transport_gate");
    fs::write(dir.join("index.js"), r#"var net = require('node:net'), tls = require('node:tls');
module.exports = async function() {
  var originalNet = globalThis.__thaw_net_poll_read, originalTls = globalThis.__thaw_tls_poll_read, netPolls = 0, tlsPolls = 0;
  globalThis.__thaw_net_poll_read = function() { netPolls++; return 'pending'; };
  globalThis.__thaw_tls_poll_read = function() { tlsPolls++; return 'pending'; };
  try {
    var plain = new net.Socket(), secure = new tls.TLSSocket();
    plain._handle = 999901; plain.readable = true;
    secure._handle = 999902; secure.readable = true; secure._readReady = true; secure._isServer = true;
    plain.pause(); secure.pause(); plain._startRead(); secure._startRead();
    await Promise.resolve();
    var userPause = netPolls === 0 && tlsPolls === 0;
    plain._setHttpReadPaused(true); secure._setHttpReadPaused(true);
    plain.resume(); secure.resume(); await Promise.resolve();
    var httpGate = netPolls === 0 && tlsPolls === 0 && !plain.isPaused() && !secure.isPaused();
    plain.pause(); secure.pause();
    plain._setHttpReadPaused(false); secure._setHttpReadPaused(false); await Promise.resolve();
    var independent = netPolls === 0 && tlsPolls === 0 && plain.isPaused() && secure.isPaused();
    plain.resume(); secure.resume(); await Promise.resolve();
    var first = netPolls === 1 && tlsPolls === 1;
    plain._setHttpReadPaused(true); secure._setHttpReadPaused(true);
    await new Promise(function(resolve) { setTimeout(resolve, 2); });
    var stopped = netPolls === 1 && tlsPolls === 1;
    plain._setHttpReadPaused(false); secure._setHttpReadPaused(false);
    await Promise.resolve();
    var restarted = netPolls === 2 && tlsPolls === 2;
    plain._handle = 0; secure._handle = 0;
    return [userPause, httpGate, independent, first, stopped, restarted];
  } finally { globalThis.__thaw_net_poll_read = originalNet; globalThis.__thaw_tls_poll_read = originalTls; }
};"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_transport_gate_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpTransportGate = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpTransportGate").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "[true,true,true,true,true,true]");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_paused_large_chunk_keeps_pipeline_behind_bounded_body() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut request = b"POST /first HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\nc000\r\n".to_vec();
        request.extend(vec![b'x'; 49152]);
        request.extend_from_slice(b"\r\n0\r\n\r\nGET /second HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        stream.write_all(&request).unwrap();
        let mut bodies = Vec::new();
        for _ in 0..2 {
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            let length = head.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length: ").and_then(|value| value.parse::<usize>().ok())).unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            bodies.push(body);
        }
        bodies
    });

    let dir = temp_registry("builtin_http_paused_large_chunk");
    fs::write(dir.join("index.js"), r#"var http = require('node:http');
module.exports = async function(port) {
  var count = 0, total = 0, snapshot, server = http.createServer(function(request, response) {
    count++;
    if (request.url === '/first') {
      request.on('data', function(chunk) { total += chunk.length; });
      request.on('end', function() { response.end('first'); });
      request.pause();
      var deadline = Date.now() + 1000; (function inspect() { if (request._bodyBytes === 16384 && request.socket._httpReadPaused) { snapshot = [request._bodyBytes, request.socket._httpReadPaused, count]; request.resume(); } else if (Date.now() < deadline) setTimeout(inspect, 1); else { snapshot = ['timeout', request._bodyBytes, count]; server.close(); } })();
    } else response.end('second', function() { server.close(); });
  });
  await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); });
  return [snapshot, total, count];
};"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_paused_large_chunk_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpPausedLargeChunk = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpPausedLargeChunk").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "[[16384,true,1],49152,2]");
    assert_eq!(client.join().unwrap(), [b"first".to_vec(), b"second".to_vec()]);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_client_large_chunk_drains_pending_before_deferred_eof_validation() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_client_pending_eof");
    fs::write(dir.join("index.js"), r#"var http = require('node:http'), EventEmitter = require('node:events');
module.exports = async function() {
  var socket = new EventEmitter(), response, total = 0, errors = [], closes = 0, gated = false;
  socket.write = function() { return true; }; socket.destroy = function() { this.emit('close'); };
  socket._setHttpReadPaused = function(value) { gated = value; };
  var transport = { createConnection: function() { return socket; } };
  var request = http.request('http://localhost/', { _transport: transport, agent: false }, function(value) {
    response = value; response.on('data', function(chunk) { total += chunk.length; });
    response.on('error', function(error) { errors.push(error.code || error.message); });
    response.pause();
  });
  request.on('error', function(error) { errors.push(error.code || error.message); });
  request.on('close', function() { closes++; });
  request.end(); socket.emit('connect');
  var packet = Buffer.concat([Buffer.from('HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nc000\r\n'), Buffer.alloc(49152, 120), Buffer.from('\r\n0\r\n\r\n')]);
  socket.emit('data', packet); socket.emit('end'); socket.emit('close');
  var before = [response._bodyBytes, gated, total, response.aborted];
  var finished = new Promise(function(resolve) { response.on('end', resolve); });
  response.resume(); await finished;
  return [before, total, response.complete, response.aborted, errors, closes];
};"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_client_pending_eof_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpClientPendingEof = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpClientPendingEof").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "[[16384,true,0,false],49152,true,false,[],1]");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_pipeline_waits_for_completed_but_unconsumed_request_body() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.write_all(b"POST /first HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1\r\n\r\nxGET /second HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n").unwrap();
        let mut bodies = Vec::new();
        for _ in 0..2 {
            let mut head = Vec::new();
            while !head.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                head.push(byte[0]);
            }
            let head = String::from_utf8(head).unwrap();
            let length = head.lines().find_map(|line| line.to_ascii_lowercase().strip_prefix("content-length: ").and_then(|value| value.parse::<usize>().ok())).unwrap();
            let mut body = vec![0; length];
            stream.read_exact(&mut body).unwrap();
            bodies.push(body);
        }
        bodies
    });

    let dir = temp_registry("builtin_http_pipeline_unconsumed");
    fs::write(dir.join("index.js"), r#"var http = require('node:http');
module.exports = async function(port) {
  var count = 0, snapshot, server = http.createServer(function(request, response) {
    count++;
    if (request.url === '/first') {
      request.on('data', function() {});
      request.pause();
      response.end('first');
      var deadline = Date.now() + 1000; (function inspect() { if (request.complete && request._bodyBytes === 1 && request.socket._httpReadPaused) { snapshot = [count, request._bodyBytes, request.socket._httpReadPaused]; request.resume(); } else if (Date.now() < deadline) setTimeout(inspect, 1); else { snapshot = ['timeout', count, request._bodyBytes]; server.close(); } })();
    } else response.end('second', function() { server.close(); });
  });
  await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); });
  return [snapshot, count];
};"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_pipeline_unconsumed_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpPipelineUnconsumed = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpPipelineUnconsumed").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "[[1,1,true],2]");
    assert_eq!(client.join().unwrap(), [b"first".to_vec(), b"second".to_vec()]);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

// A request handler may block on a nested event loop (an awaited promise), and that nested loop
// delivers the request body through the parser's resume hook while the 'request' event is still
// being emitted. The parser must accept that re-entry instead of deferring it until the handler
// returns, which the handler is itself waiting for.
#[test]
fn http_request_body_is_deliverable_while_the_request_handler_is_still_running() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let client = std::thread::spawn(move || {
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.write_all(b"POST /echo HTTP/1.1\r\nHost: localhost\r\nContent-Length: 5\r\nConnection: close\r\n\r\nhello").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });

    let dir = temp_registry("builtin_http_body_during_handler");
    fs::write(dir.join("index.js"), r#"var http = require('node:http');
module.exports = async function(port) {
  var server = http.createServer(function(request, response) {
    var received = '';
    request.on('data', function(chunk) { received += String(chunk); });
    request._bodyResume();
    request._drainBody();
    response.end('during-handler=' + received, function() { server.close(); });
  });
  await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); });
  return 'done';
};"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_body_during_handler_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpBodyDuringHandler = globalThis.module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpBodyDuringHandler").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "\"done\"");
    let response = client.join().unwrap();
    assert!(response.ends_with("during-handler=hello"), "{response}");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http_client_rejects_malformed_incremental_chunk_sizes() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http_client_invalid_chunk_size");
    fs::write(dir.join("index.js"), r#"var http = require('node:http'), EventEmitter = require('node:events');
module.exports = async function() {
  var results = [];
  for (var token of ['-1', '1junk', '20000000000000']) {
    var socket = new EventEmitter(), response, errors = 0;
    socket.write = function() { return true; }; socket.destroy = function() { this.emit('close'); };
    var request = http.request('http://localhost/', { agent: false, _transport: { createConnection: function() { return socket; } } }, function(value) {
      response = value; response.on('error', function(error) { if (error.message === 'Parse Error: Invalid chunk size') errors++; });
    });
    request.on('error', function(error) { if (error.message === 'Parse Error: Invalid chunk size') errors++; });
    request.end(); socket.emit('connect');
    socket.emit('data', Buffer.from('HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n' + token + '\r\nx\r\n0\r\n\r\n'));
    results.push(!!response && response.aborted && !response.complete && errors === 2);
  }
  return results;
};"#).unwrap();
    let empty_node_modules = temp_registry("builtin_http_client_invalid_chunk_size_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpInvalidChunkSize = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpInvalidChunkSize").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, "[true,true,true]");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn net_socket_terminal_cleanup_preserves_first_listener_error() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_net_terminal_cleanup");
    fs::write(dir.join("index.js"), r#"var net = require('node:net');
module.exports = function() {
  var originalQueue = globalThis.queueMicrotask, originalPoll = globalThis.__thaw_net_poll_read, originalDestroy = globalThis.__thaw_net_destroy, originalListen = globalThis.__thaw_net_listen, originalAccept = globalThis.__thaw_net_poll_accept, originalCloseListener = globalThis.__thaw_net_close_listener, originalTimer = globalThis.__thaw_set_timeout_ref;
  var queued = [], retired = [], marker = new Error('listener marker'), closeMarker = new Error('close marker'), nativeMarker = new Error('native marker');
  globalThis.queueMicrotask = function(task) { queued.push(task); };
  globalThis.__thaw_net_destroy = function(handle) { retired.push(handle); };
  function drain() { while (queued.length) queued.shift()(); }
  function socket(handle) { var value = new net.Socket(); value._handle = handle; value.readable = value.writable = true; return value; }
  try {
    var duplicate = socket(910001), duplicateEvents = [], duplicateCaught = false;
    duplicate.on('error', function() { duplicateEvents.push('error'); duplicate.destroy(); throw marker; });
    duplicate.on('close', function() { duplicateEvents.push('close'); duplicate.destroy(); throw closeMarker; });
    duplicate.destroy(new Error('failure')); duplicate.destroy(new Error('again'));
    try { drain(); } catch (error) { duplicateCaught = error === marker; }
    var duplicateResult = [duplicateCaught, duplicateEvents, retired.slice(), duplicate.destroyed, duplicate._handle];
    retired.length = 0;

    var readError = socket(910002), readEvents = [], readCaught = false;
    globalThis.__thaw_net_poll_read = function() { return 'err:read failed'; };
    readError.on('error', function(error) { readEvents.push('error:' + error.code); throw marker; });
    readError.on('close', function(value) { readEvents.push('close:' + value); });
    readError._startRead();
    try { drain(); } catch (error) { readCaught = error === marker; }
    var readResult = [readCaught, readEvents, readError.destroyed, readError._handle, retired.slice()];

    var eof = socket(910003), eofEvents = [], eofCaught = false;
    globalThis.__thaw_net_poll_read = function() { return 'eof'; };
    eof.on('end', function() { eofEvents.push('end'); throw marker; });
    eof.on('close', function(value) { eofEvents.push('close:' + value); });
    eof._startRead();
    try { drain(); } catch (error) { eofCaught = error === marker; }
    var eofResult = [eofCaught, eofEvents, eof.destroyed, eof._handle];

    var data = socket(910004), dataEvents = [], dataCaught = false;
    globalThis.__thaw_net_poll_read = function() { return 'ok:61'; };
    data.on('data', function() { dataEvents.push('data'); throw marker; });
    data.on('close', function() { dataEvents.push('close'); });
    data._startRead();
    try { queued.shift()(); } catch (error) { dataCaught = error === marker; }
    drain();
    var dataResult = [dataCaught, dataEvents, data.destroyed, data._handle, retired.slice()];

    var native = socket(910005), nativeEvents = [], nativeCaught = false;
    globalThis.__thaw_net_destroy = function() { throw nativeMarker; };
    native.on('close', function() { nativeEvents.push('close'); });
    try { native.destroy(); } catch (error) { nativeCaught = error === nativeMarker; }
    drain();
    var nativeResult = [nativeCaught, nativeEvents, native.destroyed, native._handle];
    var falsy = socket(910008), falsyEvents = [], falsyCaught = false;
    globalThis.__thaw_net_destroy = function() { throw undefined; };
    falsy.on('close', function() { falsyEvents.push('close'); });
    try { falsy.destroy(); } catch (error) { falsyCaught = error === undefined; }
    drain();
    var falsyResult = [falsyCaught, falsyEvents, falsy.destroyed, falsy._handle];

    retired.length = 0;
    globalThis.__thaw_net_destroy = function(handle) { retired.push(handle); };
    globalThis.__thaw_net_listen = function() { return 'ok:910006:12345'; };
    globalThis.__thaw_net_poll_accept = function() { return 'ok:910007:127.0.0.1:12346'; };
    globalThis.__thaw_net_close_listener = function() {};
    var scheduled = 0;
    globalThis.__thaw_set_timeout_ref = function() { scheduled++; return scheduled; };
    var server = net.createServer(function() { throw marker; }), acceptCaught = false;
    server.listen(0);
    try { queued.shift()(); } catch (error) { acceptCaught = error === marker; }
    server.close(); drain();
    var acceptedResult = [acceptCaught, retired.slice(), server.connections, scheduled];
    return [duplicateResult, readResult, eofResult, dataResult, nativeResult, falsyResult, acceptedResult];
  } finally {
    globalThis.queueMicrotask = originalQueue;
    globalThis.__thaw_net_poll_read = originalPoll;
    globalThis.__thaw_net_destroy = originalDestroy;
    globalThis.__thaw_net_listen = originalListen;
    globalThis.__thaw_net_poll_accept = originalAccept;
    globalThis.__thaw_net_close_listener = originalCloseListener;
    globalThis.__thaw_set_timeout_ref = originalTimer;
  }
};"#).unwrap();
    let empty_node_modules = temp_registry("builtin_net_terminal_cleanup_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetTerminalCleanup = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNetTerminalCleanup").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[[true,["error","close"],[910001],true,0],[true,["error:ECONNRESET","close:true"],true,0,[]],[true,["end","close:false"],true,0],[true,["data","close"],true,0,[910004]],[true,["close"],true,0],[true,["close"],true,0],[true,[910007],0,1]]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn net_connect_listener_throw_retires_socket_without_duplicate_terminal_events() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_net_connect_listener_throw");
    fs::write(dir.join("index.js"), r#"var net = require('node:net');
module.exports = function() {
  var originalQueue = globalThis.queueMicrotask, originalConnect = globalThis.__thaw_net_connect, originalPoll = globalThis.__thaw_net_poll_read, originalDestroy = globalThis.__thaw_net_destroy;
  var queued = [], retired = [], polls = 0, nextHandle = 920001, marker = new Error('connect listener');
  globalThis.queueMicrotask = function(task) { queued.push(task); };
  globalThis.__thaw_net_connect = function() { return 'ok:' + nextHandle++; };
  globalThis.__thaw_net_poll_read = function() { polls++; return 'pending'; };
  globalThis.__thaw_net_destroy = function(handle) { retired.push(handle); };
  function drain() { while (queued.length) queued.shift()(); }
  function probe(kind) {
    var socket = new net.Socket(), events = [], caught = false, before = retired.length, beforePolls = polls;
    socket.on('close', function(value) { events.push('close:' + value); socket.destroy(); });
    socket.on('error', function() { events.push('error'); });
    if (kind === 'before') socket.on('connect', function() { events.push('connect'); throw marker; });
    socket.connect({ port: 12345 }, kind === 'after' ? function() { events.push('connect'); throw marker; } : kind === 'reentrant' ? function() { events.push('connect'); socket.destroy(); throw marker; } : kind === 'falsy' ? function() { events.push('connect'); throw undefined; } : undefined);
    if (kind === 'immediate') socket.destroy();
    try { queued.shift()(); } catch (error) { caught = kind === 'falsy' ? error === undefined : error === marker; }
    drain();
    return [caught, events, retired.slice(before), polls - beforePolls, socket.destroyed, socket._handle];
  }
  try { return [probe('after'), probe('before'), probe('reentrant'), probe('falsy'), probe('immediate')]; }
  finally { globalThis.queueMicrotask = originalQueue; globalThis.__thaw_net_connect = originalConnect; globalThis.__thaw_net_poll_read = originalPoll; globalThis.__thaw_net_destroy = originalDestroy; }
};"#).unwrap();
    let empty_node_modules = temp_registry("builtin_net_connect_listener_throw_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNetConnectThrow = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNetConnectThrow").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[[true,["connect","close:false"],[920001],0,true,0],[true,["connect","close:false"],[920002],0,true,0],[true,["connect","close:false"],[920003],0,true,0],[true,["connect","close:false"],[920004],0,true,0],[false,["close:false"],[920005],0,true,0]]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn net_accepted_socket_keeps_write_half_only_when_requested() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    for allow_half_open in [true, false] {
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let peer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
                Err(error) => panic!("TCP connect timed out: {error}"),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        stream.write_all(b"ping").unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });
    let dir = temp_registry("builtin_net_accepted_half_close");
    fs::write(dir.join("index.js"), r#"var net = require('node:net'); module.exports = function(port, allowHalfOpen) { return new Promise(function(resolve, reject) {
      var server = net.createServer({ allowHalfOpen: allowHalfOpen }), socket, ends = 0, closes = 0, callbacks = 0, callbacksOnClose = -1, writableOnEnd = false;
      var timeout = setTimeout(function() { if (socket) socket.destroy(); server.close(); }, 2000);
      server.on('connection', function(value) { socket = value; value.on('error', reject); value.on('end', function() { ends++; writableOnEnd = value.writable; if (allowHalfOpen) setTimeout(function() { value.end('pong', function() { callbacks++; }); }, 10); else value.write('sync'); }); value.on('close', function() { closes++; callbacksOnClose = callbacks; server.close(); }); });
      server.on('error', reject); server.on('close', function() { clearTimeout(timeout); resolve([ends, closes, writableOnEnd, callbacksOnClose]); });
      server.listen(port, '127.0.0.1');
    }); };"#).unwrap();
    let modules = temp_registry("builtin_net_accepted_half_close_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; {bundle} globalThis.exerciseHalfClose = module.exports;");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(CString::new("exerciseHalfClose").unwrap().as_ptr(), CString::new(format!("[{port},{allow_half_open}]")).unwrap().as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), if allow_half_open { "[1,1,true,1]" } else { "[1,1,true,0]" });
    assert_eq!(peer.join().unwrap(), if allow_half_open { "pong" } else { "sync" });
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&modules);
    }
}

#[test]
fn http_server_finishes_buffered_pipeline_after_peer_half_close() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{Shutdown, TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let peer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut stream = loop {
            match TcpStream::connect(("127.0.0.1", port)) {
                Ok(stream) => break stream,
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
                Err(error) => panic!("TCP connect timed out: {error}"),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        stream.write_all(b"GET /first HTTP/1.1\r\nHost: localhost\r\n\r\nGET /second HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        stream.shutdown(Shutdown::Write).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });
    let dir = temp_registry("builtin_http_half_closed_pipeline");
    fs::write(dir.join("index.js"), r#"var http = require('node:http'); module.exports = function(port) { return new Promise(function(resolve, reject) {
      var server = http.createServer(), socket, requests = [], ends = 0, closes = 0;
      var timeout = setTimeout(function() { if (socket) socket.destroy(); server.close(); }, 2000);
      server.on('connection', function(value) { socket = value; value.on('error', reject); value.on('end', function() { ends++; }); value.on('close', function() { closes++; server.close(); }); });
      server.on('request', function(req, res) { requests.push(req.url); if (req.url === '/first') { res.setHeader('Content-Length', '3'); res.flushHeaders(); } setTimeout(function() { res.end(req.url === '/first' ? 'one' : 'two'); }, 10); });
      server.on('error', reject); server.on('close', function() { clearTimeout(timeout); resolve([requests, ends, closes]); });
      server.listen(port, '127.0.0.1');
    }); };"#).unwrap();
    let modules = temp_registry("builtin_http_half_closed_pipeline_modules");
    let (bundle, _, _, _) = bundle_commonjs_package(&modules, "pkg", &dir, "index.js").unwrap();
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; {bundle} globalThis.exerciseHttpHalfClose = module.exports;");
    assert_eq!(thaw_quickjs::thaw_js_load(CString::new(script).unwrap().as_ptr()), 1);
    let result = thaw_quickjs::thaw_js_call(CString::new("exerciseHttpHalfClose").unwrap().as_ptr(), CString::new(format!("[{port}]")).unwrap().as_ptr());
    assert_eq!(unsafe { CStr::from_ptr(result) }.to_string_lossy(), "[[\"/first\",\"/second\"],1,1]");
    let response = peer.join().unwrap();
    assert!(response.contains("\r\n\r\noneHTTP/1.1 200"), "{response}");
    assert!(response.ends_with("\r\n\r\ntwo"), "{response}");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&modules);
}
