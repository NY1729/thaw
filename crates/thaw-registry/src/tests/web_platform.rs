#[test]
fn vm_builtin_runs_scripts_in_contexts_and_compiles_functions() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_vm");
    fs::write(dir.join("index.js"), "var vm = require('node:vm'); module.exports = async function () { var sandbox = { value: 3 }; var context = vm.createContext(sandbox); var script = new vm.Script('value *= 4; created = value + 1; value', { filename: 'sample.js' }); var contextual = script.runInContext(context); var fresh = vm.runInNewContext('input + 2', { input: 5 }); var current = vm.runInThisContext('6 * 7'); var add = vm.compileFunction('return left + right;', ['left', 'right'], { filename: 'add.js' }); var memory = await vm.measureMemory(); var cached = script.createCachedData(); return [contextual, sandbox.value, sandbox.created, fresh, current, add(8, 9), vm.isContext(context), vm.isContext({}), cached.toString().includes('value *= 4'), script.cachedDataRejected, memory.total.jsMemoryEstimate, vm.getDefaultContext() === globalThis, typeof vm.constants.DONT_CONTEXTIFY]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_vm_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseVm = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseVm").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[12,12,13,7,42,17,true,false,true,false,0,true,"symbol"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn punycode_builtin_converts_unicode_labels_and_code_points() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_punycode");
    fs::write(dir.join("index.js"), "var punycode = require('node:punycode'); module.exports = function () { var encoded = punycode.encode('mañana'); var snowman = punycode.encode('☃-⌘'); var points = punycode.ucs2.decode('A😀Z'); return [encoded, punycode.decode(encoded), snowman, punycode.decode(snowman), punycode.toASCII('mañana.com'), punycode.toUnicode('xn--bcher-kva.example'), points, punycode.ucs2.encode(points), punycode.toASCII('user@bücher.example'), punycode.version]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_punycode_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exercisePunycode = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exercisePunycode").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["maana-pta","mañana","--dqo34k","☃-⌘","xn--maana-pta.com","bücher.example",[65,128512,90],"A😀Z","user@xn--bcher-kva.example","2.1.0"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn dgram_socket_exchanges_real_udp_datagrams() {
    use std::ffi::{CStr, CString};
    use std::net::UdpSocket;
    use std::time::Duration;

    let probe = UdpSocket::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);
    let peer = UdpSocket::bind("127.0.0.1:0").unwrap();
    peer.set_read_timeout(Some(Duration::from_millis(30)))
        .unwrap();
    let client = std::thread::spawn(move || {
        let mut response = [0u8; 16];
        loop {
            peer.send_to(b"ping", ("127.0.0.1", port)).unwrap();
            if let Ok((length, _)) = peer.recv_from(&mut response) {
                return response[..length].to_vec();
            }
        }
    });

    let dir = temp_registry("builtin_dgram");
    fs::write(dir.join("index.js"), "var dgram = require('node:dgram'); module.exports = async function (port) { var events = []; var socket = dgram.createSocket('udp4'); var closed = new Promise(function(resolve, reject) { socket.on('error', reject); socket.on('listening', function() { var address = socket.address(); events.push('listening:' + address.family + ':' + address.port); }); socket.on('message', function(message, remote) { events.push('message:' + message.toString() + ':' + remote.family + ':' + remote.size); socket.send('pong', remote.port, remote.address, function(error, written) { if (error) reject(error); else { events.push('sent:' + written); socket.close(); } }); }); socket.on('close', function() { events.push('close'); resolve(); }); }); socket.bind(port, '127.0.0.1'); await closed; return [events, socket.hasRef(), socket.ref() === socket, socket.unref() === socket]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_dgram_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseDgram = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseDgram").unwrap();
    let arguments = CString::new(format!("[{port}]")).unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        format!(
            r#"[["listening:IPv4:{port}","message:ping:IPv4:4","sent:4","close"],true,true,true]"#
        )
    );
    assert_eq!(client.join().unwrap(), b"pong");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_consumers_collect_streams_and_async_iterables() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_consumers");
    fs::write(dir.join("index.js"), "var stream = require('node:stream'); var consumers = require('node:stream/consumers'); function consume(method, value) { var input = new stream.PassThrough(); var result = consumers[method](input); input.end(value); return result; } module.exports = async function () { var text = await consume('text', 'hello'); var object = await consume('json', '{\"answer\":42}'); var bytes = await consume('buffer', 'abc'); var array = new Uint8Array(await consume('arrayBuffer', 'xy')); var blob = await consume('blob', 'blob'); var iterable = { async *[Symbol.asyncIterator]() { yield 'one'; yield Buffer.from('two'); } }; var joined = await consumers.text(iterable); var sliced = blob.slice(1, 3, 'text/plain'); return [text, object.answer, bytes.toString('hex'), Array.from(array), blob instanceof Blob, blob.size, await blob.text(), sliced.type, await sliced.text(), joined, new File(['x'], 'a.txt', { lastModified: 7 }).name]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_stream_consumers_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseConsumers = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseConsumers").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["hello",42,"616263",[120,121],true,4,"blob","text/plain","lo","onetwo","a.txt"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn stream_web_pipes_transforms_and_exposes_readers_and_writers() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_stream_web");
    fs::write(dir.join("index.js"), "var web = require('node:stream/web'); var consumers = require('node:stream/consumers'); module.exports = async function () { var source = new web.ReadableStream({ start: function(controller) { controller.enqueue('one'); controller.enqueue('two'); controller.close(); } }); var upper = new web.TransformStream({ transform: function(chunk, controller) { controller.enqueue(chunk.toUpperCase()); } }); var transformed = await consumers.text(source.pipeThrough(upper)); var writes = []; var writable = new web.WritableStream({ write: function(chunk) { writes.push(chunk); }, close: function() { writes.push('closed'); } }); var writer = writable.getWriter(); await writer.write('value'); await writer.close(); writer.releaseLock(); var encodedSource = new web.ReadableStream({ start: function(controller) { controller.enqueue('hé'); controller.close(); } }); var decoded = await consumers.text(encodedSource.pipeThrough(new web.TextEncoderStream()).pipeThrough(new web.TextDecoderStream())); var readerSource = new web.ReadableStream({ pull: function(controller) { controller.enqueue(7); controller.close(); } }); var reader = readerSource.getReader(); var first = await reader.read(); var done = await reader.read(); reader.releaseLock(); var byteStrategy = new web.ByteLengthQueuingStrategy({ highWaterMark: 8 }); var countStrategy = new web.CountQueuingStrategy({ highWaterMark: 3 }); return [web.ReadableStream === globalThis.ReadableStream, transformed, writes, decoded, first, done.done, readerSource.locked, byteStrategy.highWaterMark, byteStrategy.size(new Uint8Array(4)), countStrategy.highWaterMark, countStrategy.size('x')]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_stream_web_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseStreamWeb = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseStreamWeb").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"ONETWO",["value","closed"],"hé",{"value":7,"done":false},true,false,8,4,3,1]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn global_headers_normalize_duplicate_and_cookie_values() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_global_headers");
    fs::write(
            dir.join("index.js"),
            "module.exports = function () { function capture(action) { try { action(); } catch (error) { return [error.name, error.code]; } } var headers = new Headers([['X-B', '  one  '], ['x-a', 'a'], ['X-B', 'two'], ['set-cookie', 'a=1'], ['Set-Cookie', 'b=2']]), calls = []; headers.forEach(function(value, name, owner) { calls.push([name, value, owner === headers]); }); var clone = new Headers(headers), record = new Headers({ Z: 1, A: ['x', 'y'] }); headers.set('replace', 'first'); headers.set('replace', 'second'); headers.delete('replace'); return [[...headers], headers.get('X-B'), headers.getSetCookie(), [...headers.keys()], [...headers.values()], headers.has('X-A'), headers.get('missing'), calls, [...clone], [...record], Object.keys(headers), Object.keys(Headers.prototype), Object.prototype.toString.call(headers), capture(function() { headers.set('bad name', 'x'); }), capture(function() { headers.set('x', 'a\\nb'); }), capture(function() { new Headers([['a']]); })]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_global_headers_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHeaders = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseHeaders").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[["set-cookie","a=1"],["set-cookie","b=2"],["x-a","a"],["x-b","one, two"]],"one, two",["a=1","b=2"],["set-cookie","set-cookie","x-a","x-b"],["a=1","b=2","a","one, two"],true,null,[["set-cookie","a=1",true],["set-cookie","b=2",true],["x-a","a",true],["x-b","one, two",true]],[["set-cookie","a=1"],["set-cookie","b=2"],["x-a","a"],["x-b","one, two"]],[["a","x,y"],["z","1"]],[],["append","delete","get","has","set","getSetCookie","keys","values","entries","forEach"],"[object Headers]",["TypeError",null],["TypeError",null],["TypeError",null]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn global_response_consumes_clones_and_constructs_bodies() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_global_response");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var text = new Response('hello'), textBefore = [text.status, text.statusText, text.ok, text.type, text.url, text.redirected, text.headers.get('content-type'), text.body.constructor.name, text.bodyUsed], textValue = await text.text(), textAfter = text.bodyUsed, params = new Response(new URLSearchParams({ a: 'two words' })), paramsForm = await params.formData(), binary = new Response(new Uint8Array([1, 2, 3])), binaryBytes = Array.from(await binary.bytes()), original = new Response('clone'), clone = original.clone(), cloneValues = [await original.text(), await clone.text()], empty = new Response(null, { status: 204 }), emptyText = await empty.text(), json = Response.json({ a: 1 }), jsonValue = [json.headers.get('content-type'), await json.text()], redirect = Response.redirect('https://example.com/a', 307), failure = Response.error(), consumed = new Response('used'); await consumed.text(); var cloneError, statusError, bodyStatusError; try { consumed.clone(); } catch (error) { cloneError = error.name; } try { new Response(null, { status: 199 }); } catch (error) { statusError = error.name; } try { new Response('x', { status: 204 }); } catch (error) { bodyStatusError = error.name; } var blobResponse = new Response(new Blob(['blob'], { type: 'text/custom' })), blob = await blobResponse.blob(); return [textBefore, textValue, textAfter, params.headers.get('content-type'), paramsForm.get('a'), binaryBytes, cloneValues, emptyText, empty.bodyUsed, jsonValue, redirect.status, redirect.headers.get('location'), failure.status, failure.type, failure.ok, failure.body, cloneError, statusError, bodyStatusError, blob.type, await blob.text(), Object.prototype.toString.call(text)]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_global_response_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseResponse = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseResponse").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[200,"",true,"default","",false,"text/plain;charset=UTF-8","ReadableStream",false],"hello",true,"application/x-www-form-urlencoded;charset=UTF-8","two words",[1,2,3],["clone","clone"],"",false,["application/json","{\"a\":1}"],307,"https://example.com/a",0,"error",false,null,"TypeError","RangeError","TypeError","text/custom","blob","[object Response]"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn global_request_normalizes_inherits_and_clones_bodies() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_global_request");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { function capture(action) { try { action(); } catch (error) { return error.name; } } var controller = new AbortController(), request = new Request('https://example.com/a?x=1', { method: 'post', headers: { X: ' y ' }, body: 'hello', signal: controller.signal }), before = [request.url, request.method, [...request.headers], request.body.constructor.name, request.bodyUsed, request.cache, request.credentials, request.destination, request.integrity, request.keepalive, request.mode, request.redirect, request.referrer, request.referrerPolicy, request.duplex, request.signal === controller.signal], clone = request.clone(), cloneText = await clone.text(), originalStillUnused = !request.bodyUsed, inherited = new Request(request, { method: 'PUT', headers: { Z: '1' } }), originalTransferred = request.bodyUsed && request.body.locked, inheritedText = await inherited.text(); controller.abort('stop'); await Promise.resolve(); var stream = new ReadableStream({ start: function(value) { value.enqueue(new TextEncoder().encode('stream')); value.close(); } }), streamRequest = new Request('http://example.com/', { method: 'POST', body: stream, duplex: 'half' }), streamText = await streamRequest.text(), params = new Request('http://example.com/', { method: 'POST', body: new URLSearchParams({ a: 'b' }) }), form = await params.formData(); return [before, cloneText, originalStillUnused, inherited.method, inherited.url, [...inherited.headers], inheritedText, originalTransferred, request.signal.aborted, request.signal.reason, inherited.signal.aborted, streamText, form.get('a'), Object.prototype.toString.call(request), capture(function() { new Request('/relative'); }), capture(function() { new Request('http://example.com/', { body: 'x' }); }), capture(function() { new Request('http://example.com/', { method: 'HEAD', body: 'x' }); }), capture(function() { new Request('http://example.com/', { method: 'bad method' }); }), capture(function() { new Request('http://example.com/', { method: 'CONNECT' }); }), capture(function() { new Request('http://example.com/', { method: 'POST', body: new ReadableStream() }); })]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_global_request_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseRequest = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseRequest").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["https://example.com/a?x=1","POST",[["content-type","text/plain;charset=UTF-8"],["x","y"]],"ReadableStream",false,"default","same-origin","","",false,"cors","follow","about:client","","half",false],"hello",true,"PUT","https://example.com/a?x=1",[["z","1"]],"hello",true,true,"stop",true,"stream","b","[object Request]","TypeError","TypeError","TypeError","TypeError","TypeError","TypeError"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn text_decoder_stream_decodes_incrementally_and_flushes_errors() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_text_decoder_streaming");
    fs::write(
            dir.join("index.js"),
            "var web = require('node:stream/web'); module.exports = async function () { var decoder = new web.TextDecoderStream(), writer = decoder.writable.getWriter(), reader = decoder.readable.getReader(), settled = false, pending = reader.read().then(function(result) { settled = true; return result; }); await writer.write(Uint8Array.from([0xf0, 0x9f])); await Promise.resolve(); await Promise.resolve(); var incompletePending = !settled; await writer.write(Uint8Array.from([0x98, 0x80, 0x41])); var decoded = await pending; await writer.close(); var done = await reader.read(), fatal = new TextDecoderStream('utf-8', { fatal: true }), fatalWriter = fatal.writable.getWriter(), fatalReader = fatal.readable.getReader(), fatalRead = fatalReader.read().catch(function(error) { return error; }); await fatalWriter.write(Uint8Array.from([0xe2])); var closeError = await fatalWriter.close().catch(function(error) { return error; }), readError = await fatalRead, closedError = await fatalWriter.closed.catch(function(error) { return error; }); return [incompletePending, decoded.value, done.done, closeError instanceof TypeError, readError === closeError, closedError === closeError, decoder.encoding, decoder.fatal, decoder.ignoreBOM]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_text_decoder_streaming_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTextDecoderStreaming = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseTextDecoderStreaming")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"😀A",true,true,true,true,"utf-8",false,false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn text_encoder_stream_preserves_split_surrogate_pairs() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_text_encoder_surrogates");
    fs::write(
            dir.join("index.js"),
            "var web = require('node:stream/web'); async function encodeParts(parts) { var stream = new web.TextEncoderStream(), writer = stream.writable.getWriter(), reader = stream.readable.getReader(), output = [], consume = (async function() { while (true) { var result = await reader.read(); if (result.done) break; output.push(Array.from(result.value)); } })(); for (var part of parts) await writer.write(part); await writer.close(); await consume; return output; } module.exports = async function () { var encoder = new TextEncoder(), destination = new Uint8Array(4), into = encoder.encodeInto('\\ud83dA', destination); return [Array.from(encoder.encode('\\ud83d')), Array.from(encoder.encode('\\ude00')), into, Array.from(destination), await encodeParts(['\\ud83d', '\\ude00A']), await encodeParts(['X\\ud83d']), await encodeParts(['\\ud83d', 'B'])]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_text_encoder_surrogates_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTextEncoderSurrogates = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseTextEncoderSurrogates")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[239,191,189],[239,191,189],{"read":2,"written":4},[239,191,189,65],[[240,159,152,128,65]],[[88],[239,191,189]],[[239,191,189,66]]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_compression_streams_round_trip_supported_formats() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_compression");
    fs::write(dir.join("index.js"), "var web = require('node:stream/web'); var consumers = require('node:stream/consumers'); async function roundTrip(format) { var source = new web.ReadableStream({ start: function(controller) { controller.enqueue(new TextEncoder().encode('thaw ')); controller.enqueue(new TextEncoder().encode('compression')); controller.close(); } }); return consumers.text(source.pipeThrough(new web.CompressionStream(format)).pipeThrough(new web.DecompressionStream(format)).pipeThrough(new web.TextDecoderStream())); } module.exports = async function () { var gzipSource = new web.ReadableStream({ start: function(controller) { controller.enqueue(new TextEncoder().encode('header')); controller.close(); } }); var gzip = await consumers.buffer(gzipSource.pipeThrough(new CompressionStream('gzip'))); var unsupported = false; try { new CompressionStream('brotli'); } catch (error) { unsupported = error instanceof TypeError; } return [await roundTrip('gzip'), await roundTrip('deflate'), await roundTrip('deflate-raw'), await roundTrip('br'), gzip[0], gzip[1], web.CompressionStream === globalThis.CompressionStream, unsupported]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_web_compression_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseCompressionStreams = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseCompressionStreams").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["thaw compression","thaw compression","thaw compression","thaw compression",31,139,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn web_compression_streams_emit_and_decode_incrementally() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_web_compression_incremental");
    fs::write(
            dir.join("index.js"),
            "module.exports = async function () { var encoder = new TextEncoder(), compression = new CompressionStream('gzip'), compressionWriter = compression.writable.getWriter(), compressionReader = compression.readable.getReader(), firstCompressedRead = compressionReader.read(); await compressionWriter.write(encoder.encode('hello')); var firstCompressed = await firstCompressedRead, compressed = [firstCompressed.value], collectCompressed = (async function() { while (true) { var result = await compressionReader.read(); if (result.done) break; compressed.push(result.value); } })(); await compressionWriter.write(encoder.encode(' world')); await compressionWriter.close(); await collectCompressed; var decompression = new DecompressionStream('gzip'), decompressionWriter = decompression.writable.getWriter(), decompressionReader = decompression.readable.getReader(), decoded = [], collectDecoded = (async function() { while (true) { var result = await decompressionReader.read(); if (result.done) break; decoded.push(result.value); } })(); var decodedBeforeClose = false; for (var index = 0; index < compressed.length; index++) { await decompressionWriter.write(compressed[index]); if (index === 0) { await new Promise(function(resolve) { setTimeout(resolve, 0); }); decodedBeforeClose = decoded.length > 0; } } await decompressionWriter.close(); await collectDecoded; var total = decoded.reduce(function(sum, chunk) { return sum + chunk.byteLength; }, 0), bytes = new Uint8Array(total), offset = 0; decoded.forEach(function(chunk) { bytes.set(chunk, offset); offset += chunk.byteLength; }); var cancelReason = new Error('stop'), cancelledWith, transform = new TransformStream({ cancel: function(reason) { cancelledWith = reason; } }), transformWriter = transform.writable.getWriter(); await transform.readable.cancel(cancelReason); var writerError = await transformWriter.closed.catch(function(error) { return error; }); return [firstCompressed.value.byteLength > 0, compressed.length > 1, decodedBeforeClose, new TextDecoder().decode(bytes), cancelledWith === cancelReason, writerError === cancelReason]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_web_compression_incremental_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 1);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseCompressionIncremental = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseCompressionIncremental")
            .unwrap()
            .as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,true,true,"hello world",true,true]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn tls_client_verifies_custom_ca_and_exchanges_encrypted_bytes() {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    use rustls::server::WebPkiClientVerifier;
    use rustls::{RootCertStore, ServerConfig, ServerConnection, StreamOwned};
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::Command;
    use std::sync::Arc;

    let certificate_dir = temp_registry("builtin_tls_certificate");
    let key_pem = certificate_dir.join("key.pem");
    let cert_pem = certificate_dir.join("cert.pem");
    let key_der = certificate_dir.join("key.der");
    let cert_der = certificate_dir.join("cert.der");
    let client_key_pem = certificate_dir.join("client-key.pem");
    let client_cert_pem = certificate_dir.join("client-cert.pem");
    let client_cert_der = certificate_dir.join("client-cert.der");
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
            "-keyout"
        ])
        .arg(&key_pem)
        .arg("-out")
        .arg(&cert_pem)
        .output()
        .unwrap()
        .status
        .success());
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
            "/CN=thaw-client",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature",
            "-addext",
            "extendedKeyUsage=clientAuth",
            "-keyout",
        ])
        .arg(&client_key_pem)
        .arg("-out")
        .arg(&client_cert_pem)
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
        .args(["x509", "-in"])
        .arg(&client_cert_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&client_cert_der)
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
    let certificate_bytes = fs::read(&cert_der).unwrap();
    let certificate = CertificateDer::from(certificate_bytes.clone());
    let private_key = PrivatePkcs8KeyDer::from(fs::read(&key_der).unwrap()).into();
    let mut client_roots = RootCertStore::empty();
    client_roots
        .add(CertificateDer::from(fs::read(&client_cert_der).unwrap()))
        .unwrap();
    let client_verifier = WebPkiClientVerifier::builder(Arc::new(client_roots))
        .build()
        .unwrap();
    let mut config = ServerConfig::builder()
        .with_client_cert_verifier(client_verifier)
        .with_single_cert(vec![certificate], private_key)
        .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    let config = Arc::new(config);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        for index in 0..2 {
            let (socket, _) = listener.accept().unwrap();
            let connection = ServerConnection::new(Arc::clone(&config)).unwrap();
            let mut stream = StreamOwned::new(connection, socket);
            let mut request = Vec::new();
            stream.read_to_end(&mut request).unwrap();
            assert_eq!(
                request,
                if index == 0 {
                    b"ping".as_slice()
                } else {
                    b"insecure".as_slice()
                }
            );
            stream
                .write_all(if index == 0 {
                    b"pong".as_slice()
                } else {
                    b"accepted".as_slice()
                })
                .unwrap();
            stream.conn.send_close_notify();
            stream.flush().unwrap();
        }
    });
    let ca_hex = fs::read(&cert_pem)
        .unwrap()
        .iter()
        .fold(String::new(), |mut output, byte| {
            use std::fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
            output
        });
    let client_cert_hex =
        fs::read(&client_cert_pem)
            .unwrap()
            .iter()
            .fold(String::new(), |mut output, byte| {
                use std::fmt::Write as _;
                let _ = write!(output, "{byte:02x}");
                output
            });
    let client_key_hex =
        fs::read(&client_key_pem)
            .unwrap()
            .iter()
            .fold(String::new(), |mut output, byte| {
                use std::fmt::Write as _;
                let _ = write!(output, "{byte:02x}");
                output
            });

    let dir = temp_registry("builtin_tls");
    fs::write(dir.join("index.js"), "var tls = require('node:tls'); module.exports = async function (port, caHex, certHex, keyHex) { var events = []; var socket = new tls.TLSSocket(); socket.on('connect', function() { events.push('connect'); }); socket.on('secureConnect', function() { events.push('secureConnect'); }); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); }); socket.on('end', function() { events.push('end'); }); var closed = new Promise(function(resolve, reject) { socket.on('error', reject); socket.on('close', function(hadError) { events.push('close:' + hadError); resolve(); }); }); socket.connect({ host: '127.0.0.1', port: port, servername: 'localhost', ca: Buffer.from(caHex, 'hex'), cert: Buffer.from(certHex, 'hex'), key: Buffer.from(keyHex, 'hex'), ALPNProtocols: ['h2', 'http/1.1'] }); socket.end('ping'); await closed; var peer = socket.getPeerCertificate(), local = socket.getCertificate(); var bypassData = '', bypass = new tls.TLSSocket(); var bypassClosed = new Promise(function(resolve, reject) { bypass.on('error', reject); bypass.on('data', function(chunk) { bypassData += chunk.toString(); }); bypass.on('close', resolve); }); bypass.connect({ host: '127.0.0.1', port: port, servername: 'not-localhost', cert: Buffer.from(certHex, 'hex'), key: Buffer.from(keyHex, 'hex'), rejectUnauthorized: false }); bypass.end('insecure'); await bypassClosed; return [events, socket.authorized, socket.authorizationError, socket.encrypted, socket.alpnProtocol, socket.getProtocol(), socket.getCipher().version, socket.bytesWritten, socket.bytesRead, tls.getCiphers().length, tls.DEFAULT_MIN_VERSION, peer.raw.length > 0, local.raw.length > 0, /^(?:[0-9A-F]{2}:){31}[0-9A-F]{2}$/.test(peer.fingerprint256), peer.subject.CN, peer.issuer.CN, local.subject.CN, peer.valid_from.length > 0, peer.valid_to.length > 0, peer.serialNumber.length > 0, bypassData, bypass.authorized, bypass.authorizationError]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_tls_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTls = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseTls").unwrap();
    let arguments = CString::new(format!(
        "[{port},\"{ca_hex}\",\"{client_cert_hex}\",\"{client_key_hex}\"]"
    ))
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["connect","secureConnect","data:pong","end","close:false"],true,null,true,"h2","TLSv1.3","TLSv1.3",4,4,3,"TLSv1.2",true,true,true,"localhost","localhost","thaw-client",true,true,true,"accepted",false,"UNABLE_TO_VERIFY_LEAF_SIGNATURE"]"#
    );
    server.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    let _ = fs::remove_dir_all(&certificate_dir);
}

#[test]
fn tls_server_accepts_verified_clients_and_exchanges_encrypted_bytes() {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer, ServerName};
    use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;

    let certificate_dir = temp_registry("builtin_tls_server_certificate");
    let key_pem = certificate_dir.join("key.pem");
    let cert_pem = certificate_dir.join("cert.pem");
    let cert_der = certificate_dir.join("cert.der");
    let client_key_pem = certificate_dir.join("client-key.pem");
    let client_cert_pem = certificate_dir.join("client-cert.pem");
    let client_key_der = certificate_dir.join("client-key.der");
    let client_cert_der = certificate_dir.join("client-cert.der");
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
        .args([
            "req",
            "-x509",
            "-newkey",
            "rsa:2048",
            "-nodes",
            "-days",
            "1",
            "-subj",
            "/CN=thaw-client",
            "-addext",
            "basicConstraints=critical,CA:FALSE",
            "-addext",
            "keyUsage=critical,digitalSignature",
            "-addext",
            "extendedKeyUsage=clientAuth",
            "-keyout",
        ])
        .arg(&client_key_pem)
        .arg("-out")
        .arg(&client_cert_pem)
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
        .args(["x509", "-in"])
        .arg(&client_cert_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&client_cert_der)
        .status()
        .unwrap()
        .success());
    assert!(Command::new("openssl")
        .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
        .arg(&client_key_pem)
        .args(["-outform", "DER", "-out"])
        .arg(&client_key_der)
        .status()
        .unwrap()
        .success());
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(fs::read(&cert_der).unwrap()))
        .unwrap();
    let mut config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(
            vec![CertificateDer::from(fs::read(&client_cert_der).unwrap())],
            PrivatePkcs8KeyDer::from(fs::read(&client_key_der).unwrap()).into(),
        )
        .unwrap();
    config.alpn_protocols = vec![b"h2".to_vec()];
    let config = Arc::new(config);
    let client = std::thread::spawn(move || {
        (0..3)
            .map(|index| {
                let socket = (0..50)
                    .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                        Ok(socket) => Some(socket),
                        Err(_) => {
                            std::thread::sleep(Duration::from_millis(10));
                            None
                        }
                    })
                    .expect("TLS server did not start");
                let connection = ClientConnection::new(
                    Arc::clone(&config),
                    ServerName::try_from("localhost".to_string()).unwrap(),
                )
                .unwrap();
                let mut stream = StreamOwned::new(connection, socket);
                stream.write_all(format!("ping{index}").as_bytes()).unwrap();
                stream.conn.send_close_notify();
                stream.flush().unwrap();
                let mut response = Vec::new();
                stream.read_to_end(&mut response).unwrap();
                response
            })
            .collect::<Vec<_>>()
    });
    let to_hex = |value: &[u8]| {
        value.iter().fold(String::new(), |mut output, byte| {
            use std::fmt::Write as _;
            let _ = write!(output, "{byte:02x}");
            output
        })
    };
    let cert_hex = to_hex(&fs::read(&cert_pem).unwrap());
    let key_hex = to_hex(&fs::read(&key_pem).unwrap());
    let client_ca_hex = to_hex(&fs::read(&client_cert_pem).unwrap());
    let dir = temp_registry("builtin_tls_server");
    fs::write(dir.join("index.js"), "var tls = require('node:tls'); module.exports = async function(port, certHex, keyHex, clientCaHex) { var events = [], handled = 0, server; var closed = new Promise(function(resolve, reject) { server = tls.createServer({ cert: Buffer.from(certHex, 'hex'), key: Buffer.from(keyHex, 'hex'), ca: Buffer.from(clientCaHex, 'hex'), requestCert: true, rejectUnauthorized: true, ALPNProtocols: ['h2', 'http/1.1'] }, function(socket) { var peer = socket.getPeerCertificate(), local = socket.getCertificate(); events.push('secureConnection:' + socket.alpnProtocol + ':' + (peer.raw.length > 0) + ':' + (local.raw.length > 0)); socket.on('error', reject); socket.on('data', function(chunk) { events.push('data:' + chunk.toString()); handled++; socket.end('pong', function() { if (handled === 3) server.close(); }); }); }); server.on('listening', function() { events.push('listening'); }); server.on('error', reject); server.on('tlsClientError', reject); server.on('close', function() { events.push('close'); resolve(); }); server.listen(port, '127.0.0.1'); }); await closed; return [events, server.listening, server.address().port, server.connections]; };").unwrap();
    let empty_node_modules = temp_registry("builtin_tls_server_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTlsServer = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseTlsServer").unwrap();
    let arguments = CString::new(format!(
        "[{port},\"{cert_hex}\",\"{key_hex}\",\"{client_ca_hex}\"]"
    ))
    .unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        format!(
            r#"[["listening","secureConnection:h2:true:true","data:ping0","secureConnection:h2:true:true","data:ping1","secureConnection:h2:true:true","data:ping2","close"],false,{port},0]"#
        )
    );
    assert_eq!(client.join().unwrap(), vec![b"pong", b"pong", b"pong"]);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    let _ = fs::remove_dir_all(&certificate_dir);
}

