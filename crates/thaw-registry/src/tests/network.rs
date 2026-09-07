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
    assert_eq!(file_count, 4);
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
fn http_client_requests_and_parses_a_real_chunked_response() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
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
    assert_eq!(file_count, 4);
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
fn http_client_emits_informational_responses_and_parses_trailers() {
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
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
    assert_eq!(file_count, 4);
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
    let server = std::thread::spawn(move || {
        for expected in [
            "GET /redirect ",
            "GET /final ",
            "POST /echo ",
            "POST /multipart ",
            "POST /post-redirect ",
            "GET /post-final ",
            "GET /gzip ",
            "GET /deflate ",
            "GET /br ",
        ] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            stream.read_to_end(&mut request).unwrap();
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with(expected), "{request}");
            if expected.contains("post-redirect") {
                assert!(request.ends_with("again"));
                stream
                        .write_all(b"HTTP/1.1 302 Found\r\nLocation: /post-final#ignored\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .unwrap();
            } else if expected.contains("redirect") {
                stream
                        .write_all(b"HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .unwrap();
            } else if expected == "GET /final " {
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
            "module.exports = async function (port) { var base = 'http://127.0.0.1:' + port; var redirected = await fetch(base + '/redirect'), cookies = redirected.headers.getSetCookie(), json = await redirected.json(); var posted = await fetch(new Request(base + '/echo', { method: 'POST', headers: { 'X-Thaw': 'enabled' }, body: 'payload' })), before = posted.bodyUsed, text = await posted.text(); var data = new FormData(); data.append('title', 'thaw'); data.append('asset', new Blob(['file-body'], { type: 'text/plain' }), 'note.txt'); var multipartRequest = new Request(base + '/multipart', { method: 'POST', body: data }), parsedRequest = await multipartRequest.clone().formData(), multipart = await fetch(multipartRequest), parsed = await multipart.formData(), upload = parsed.get('upload'); var rewritten = await fetch(base + '/post-redirect#source', { method: 'POST', body: 'again' }), rewrittenText = await rewritten.text(), gzip = await fetch(base + '/gzip'), gzipText = await gzip.text(), deflate = await fetch(base + '/deflate'), deflateText = await deflate.text(), br = await fetch(base + '/br'), brText = await br.text(); var controller = new AbortController(), abortReason; controller.abort('stop'); try { await fetch(base + '/unused', { signal: controller.signal }); } catch (error) { abortReason = error; } var schemeError; try { await fetch('file:///tmp/value'); } catch (error) { schemeError = error instanceof TypeError; } return [redirected.status, redirected.ok, redirected.redirected, redirected.url, cookies, json.ok, posted.status, posted.statusText, posted.headers.get('content-type'), before, posted.bodyUsed, text, typeof fetch, abortReason, schemeError, parsedRequest.get('title'), await parsedRequest.get('asset').text(), parsed.get('answer'), upload instanceof File, upload.name, upload.type, await upload.text(), rewritten.redirected, rewritten.url, rewrittenText, gzipText, gzip.headers.get('content-encoding'), deflateText, deflate.headers.get('content-encoding'), brText, br.headers.get('content-encoding')]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("global_fetch_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 6);
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
            r#"[200,true,true,"http://127.0.0.1:{port}/final",["a=1","b=2"],true,201,"Created","text/plain",false,true,"payload","function","stop",true,"thaw","file-body","42",true,"reply.txt","text/plain","reply-body",true,"http://127.0.0.1:{port}/post-final","rewritten","compressed","gzip","compressed","deflate","compressed","br"]"#
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
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
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
        let mut request = Vec::new();
        stream.read_to_end(&mut request).unwrap();
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
    assert_eq!(file_count, 6);
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
            r#"var cp = require('node:child_process'); module.exports = async function (childPath) { var child = cp.fork(childPath, ['argument'], { silent: true, execArgv: ['--no-warnings'] }), events = [], output = '', callbackCode; child.stdout.on('data', function(value) { output += value.toString(); }); var completed = new Promise(function(resolve, reject) { child.on('error', reject); child.on('spawn', function() { events.push('spawn'); child.send({ base: 40 }, function(error) { callbackCode = error && error.code || null; }); }); child.on('message', function(value) { events.push('message:' + value.answer + ':' + value.argument); }); child.on('disconnect', function() { events.push('disconnect'); }); child.on('close', function(code, signal) { events.push('close:' + code + ':' + signal); resolve(); }); }); await completed; var closed; try { child.send({ late: true }); } catch (error) { closed = error.code; } return [events, output.trim(), callbackCode, child.connected, child.channel, closed, child instanceof cp.ChildProcess]; };"#,
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
                .write_all(b"POST /submit HTTP/1.1\r\nHost: localhost\r\nX-Client: rust\r\nContent-Length: 4\r\nConnection: close\r\n\r\nping")
                .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });

    let dir = temp_registry("builtin_http_server");
    fs::write(
            dir.join("index.js"),
            "var http = require('node:http'); module.exports = async function (port) { var observed = []; var server = http.createServer(function(request, response) { var body = []; observed.push(request.method, request.url, request.headers['x-client'], request.httpVersion); request.on('data', function(chunk) { body.push(chunk.toString()); }); request.on('end', function() { observed.push(body.join('')); response.statusCode = 201; response.statusMessage = 'Stored'; response.setHeader('X-Server', 'thaw'); response.setHeader('Set-Cookie', ['a=1', 'b=2']); response.write('po'); response.end('ng', function() { server.close(); }); }); }); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return [observed, server.listening, server instanceof http.Server]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http_server_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpServer = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpServer").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["POST","/submit","rust","1.1","ping"],false,true]"#
    );
    let response = client.join().unwrap();
    assert!(response.starts_with("HTTP/1.1 201 Stored\r\n"));
    assert!(response.contains("X-Server: thaw\r\n"));
    assert!(response.contains("Set-Cookie: a=1\r\nSet-Cookie: b=2\r\n"));
    assert!(response.ends_with("\r\n\r\npong"));
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
    let server = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let connection = ServerConnection::new(config).unwrap();
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
    assert_eq!(file_count, 6);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttpsClient = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttpsClient").unwrap();
    let ca = fs::read_to_string(&cert_pem).unwrap();
    let arguments = CString::new(serde_json::to_string(&(port, ca)).unwrap()).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[200,"yes","secret","https:",true,true,"https:"]"#
    );
    server.join().unwrap();

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
        let socket = loop {
            match TcpStream::connect(("127.0.0.1", server_port)) {
                Ok(socket) => break socket,
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(2)),
            }
        };
        let connection =
            ClientConnection::new(client_config, ServerName::try_from("localhost").unwrap())
                .unwrap();
        let mut stream = StreamOwned::new(connection, socket);
        stream
            .write_all(b"GET /from-rust HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
            .unwrap();
        stream.conn.send_close_notify();
        stream.flush().unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        response
    });
    let server_dir = temp_registry("builtin_https_server");
    fs::write(
            server_dir.join("index.js"),
            "var https = require('node:https'); module.exports = async function (port, cert, key) { var observed; var server = https.createServer({ cert: cert, key: key }, function(request, response) { observed = [request.method, request.url, request.socket.encrypted, request.socket.authorized]; response.statusCode = 202; response.setHeader('X-TLS', 'yes'); response.end('secure-server', function() { server.close(); }); }); await new Promise(function(resolve, reject) { server.on('error', reject); server.on('close', resolve); server.listen(port, '127.0.0.1'); }); return [observed, server.listening, server instanceof https.Server]; };",
        )
        .unwrap();
    let server_node_modules = temp_registry("builtin_https_server_node_modules");
    let (server_bundle, _, server_file_count, _) =
        bundle_commonjs_package(&server_node_modules, "secure-pkg", &server_dir, "index.js")
            .unwrap();
    assert_eq!(server_file_count, 6);
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
        r#"[["GET","/from-rust",true,true],false,true]"#
    );
    let response = tls_client.join().unwrap();
    assert!(response.starts_with("HTTP/1.1 202 Accepted\r\n"));
    assert!(response.contains("X-TLS: yes\r\n"));
    assert!(response.ends_with("\r\n\r\nsecure-server"));
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
    fs::write(dir.join("index.js"), "var net = require('node:net'); module.exports = async function (port) { var events = [], handled = 0; var server; var closed = new Promise(function(resolve, reject) { server = net.createServer(function(socket) { events.push('connection:' + socket.remoteFamily); socket.on('error', reject); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); handled++; socket.end('pong' + handled, function() { if (handled === 3) server.close(); }); }); }); server.on('error', reject); server.on('listening', function() { var address = server.address(); events.push('listening:' + address.address + ':' + address.port); }); server.on('close', function() { events.push('close'); resolve(); }); server.listen(port, '127.0.0.1'); }); await closed; return [events, server.listening, server.address().port, server.connections, server.ref() === server, server.unref() === server]; };").unwrap();
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
