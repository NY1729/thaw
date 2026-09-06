/// The exact pattern found in a real ESM npm package (`has-flag`):
/// `import process from 'process'`, then reading `process.argv`.
#[test]
fn process_builtin_polyfill_actually_runs_through_quickjs() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_process");
    fs::write(
        dir.join("index.js"),
        "import process from 'process';\n\
             export default function getPlatform() { return [process.platform, process.arch]; }",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_process_node_modules");

    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();

    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.getPlatform = module.exports.default;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source.as_ptr()),
        1,
        "failed to load"
    );

    let func = CString::new("getPlatform").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    let expected_arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        value => value,
    };
    assert_eq!(result, format!(r#"["linux","{expected_arch}"]"#));

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn url_builtin_supports_legacy_parse_and_format() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_url_legacy");
    fs::write(
        dir.join("index.js"),
        "var url = require('url'); module.exports = function () { var relative = url.parse('/route?q=1'); var absolute = url.parse('https://user:pass@example.com:8443/a?q=2#h'); return [relative.pathname, relative.path, relative.host, absolute.protocol, absolute.hostname, absolute.port, url.format(relative), url.format(absolute)]; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_url_legacy_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
        "globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseUrl = module.exports;"
    );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseUrl").unwrap();
    let args = CString::new("[]").unwrap();
    let result = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["/route","/route?q=1",null,"https:","example.com","8443","/route?q=1","https://user:pass@example.com:8443/a?q=2#h"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn querystring_builtin_runs_through_bundled_commonjs_require() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_querystring");
    fs::write(
            dir.join("index.js"),
            "var querystring = require('querystring');\n\
             module.exports = function () {\n\
             \x20 return querystring.stringify({ a: [1, 2], space: 'two words' }) + ':' + JSON.stringify(querystring.parse('x=1&x=2'));\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_querystring_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);

    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseQuerystring = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseQuerystring").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#""a=1&a=2&space=two%20words:{\"x\":[\"1\",\"2\"]}""#
    );

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn events_builtin_runs_event_emitter_through_commonjs_require() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_events");
    fs::write(
            dir.join("index.js"),
            "var EventEmitter = require('events');\n\
             function Child() { EventEmitter.call(this); }\n\
             Child.prototype = Object.create(EventEmitter.prototype);\n\
             Child.prototype.constructor = Child;\n\
             function LazyChild() {}\n\
             LazyChild.prototype = Object.create(EventEmitter.prototype);\n\
             module.exports = function () {\n\
             \x20 var emitter = new Child(); var seen = [];\n\
             \x20 function regular(value) { seen.push('regular:' + value); }\n\
             \x20 emitter.on('value', regular);\n\
             \x20 emitter.prependOnceListener('value', function(value) { seen.push('once:' + value); });\n\
             \x20 var first = emitter.emit('value', 1); var second = emitter.emit('value', 2);\n\
             \x20 emitter.off('value', regular); var third = emitter.emit('value', 3);\n\
             \x20 var lazy = new LazyChild(), lazySeen = 0; lazy.on('value', function() { lazySeen++; }); lazy.emit('value');\n\
             \x20 return [seen.join(','), first, second, third, emitter.listenerCount('value'), emitter.eventNames().length, lazySeen];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_events_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);

    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseEvents = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseEvents").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["once:1,regular:1,regular:2",true,true,false,0,0,1]"#
    );

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn events_builtin_supports_symbols_async_iteration_and_abort() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_events_async");
    fs::write(
            dir.join("index.js"),
            "var events = require('node:events'); module.exports = async function () { var emitter = new events.EventEmitter(), symbol = Symbol('value'), observed = []; emitter.on(symbol, function(value) { observed.push(value); }); emitter.emit(symbol, 1); var iterator = events.on(emitter, 'data'); emitter.emit('data', 'first', 2); emitter.emit('data', 'second', 3); var first = await iterator.next(), second = await iterator.next(), returned = await iterator.return(); var controller = new AbortController(), aborted = events.on(emitter, 'wait', { signal: controller.signal }), error; var pending = aborted.next().catch(function(value) { error = [value.name, value.code, value.cause]; }); controller.abort('reason'); await pending; var onceController = new AbortController(), onceError; var oncePending = events.once(emitter, 'never', { signal: onceController.signal }).catch(function(value) { onceError = [value.name, value.code, value.cause]; }); onceController.abort('once-reason'); await oncePending; events.setMaxListeners(4, emitter); var listener = function() {}; emitter.on('probe', listener); return [observed, first.value, second.value, returned.done, emitter.listenerCount('data'), emitter.eventNames().some(function(value) { return value === symbol; }), events.getEventListeners(emitter, 'probe')[0] === listener, emitter.getMaxListeners(), events.getMaxListeners(emitter), error, onceError, typeof events.errorMonitor, typeof events.captureRejectionSymbol]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_events_async_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseEventsAsync = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseEventsAsync").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[[1],["first",2],["second",3],true,0,true,true,4,4,["AbortError","ABORT_ERR","reason"],["AbortError","ABORT_ERR","once-reason"],"symbol","symbol"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn assert_builtin_reports_structured_failures_through_commonjs_require() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_assert");
    fs::write(
            dir.join("index.js"),
            "var assert = require('node:assert/strict');\n\
             module.exports = function () {\n\
             \x20 assert.deepStrictEqual({ a: [1, 2], date: new Date(3) }, { date: new Date(3), a: [1, 2] });\n\
             \x20 assert.throws(function() { throw new TypeError('bad value'); }, /bad/);\n\
             \x20 var failure; try { assert.strictEqual(1, 2, 'different'); } catch (error) { failure = [error instanceof assert.AssertionError, error.name, error.code, error.actual, error.expected, error.operator, error.message]; }\n\
             \x20 return failure;\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_assert_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);

    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseAssert = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseAssert").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"AssertionError","ERR_ASSERTION",1,2,"strictEqual","different"]"#
    );

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

/// Not just plausible text -- runs through the real QuickJS-NG
/// engine, confirming `util.inspect.custom` actually comes back as a
/// real `Symbol` (what `object-inspect` needs it to be), not merely
/// present.
#[test]
fn util_polyfill_actually_runs_through_quickjs() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_util_runs");
    fs::write(
        dir.join("index.js"),
        "var inspect = require('util').inspect;\n\
             module.exports = function () { return typeof inspect.custom; };",
    )
    .unwrap();
    let empty_node_modules = temp_registry("builtin_util_runs_node_modules");

    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();

    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.checkInspectCustom = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(
        thaw_quickjs::thaw_js_load(source.as_ptr()),
        1,
        "bundle failed to load"
    );

    let func = CString::new("checkInspectCustom").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    assert_eq!(result, "\"symbol\"");

    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn util_helpers_run_through_bundled_commonjs_require() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_util_helpers");
    fs::write(
            dir.join("index.js"),
            "var util = require('node:util');\n\
             function Base() {} function Child() {} util.inherits(Child, Base);\n\
             module.exports = async function () {\n\
             \x20 var custom = {}; custom[util.inspect.custom] = function() { return 'custom'; };\n\
             \x20 var add = util.promisify(function(a, b, callback) { queueMicrotask(function() { callback(null, a + b); }); });\n\
             \x20 var sum = await add(2, 3);\n\
             \x20 var callbackValue = await new Promise(function(resolve, reject) { util.callbackify(async function(value) { return value * 2; })(4, function(error, value) { if (error) reject(error); else resolve(value); }); });\n\
             \x20 var debug = util.debuglog('sharp');\n\
             \x20 return [util.format('%s:%d:%j:%%', 'value', 4, { ok: true }), util.inspect(custom), Child.super_ === Base, new Child() instanceof Base, util.types.isDate(new Date()), util.types.isRegExp(/x/), util.types.isMap(new Map()), util.types.isTypedArray(new Uint8Array(1)), sum, callbackValue, util.stripVTControlCharacters('\\u001b[31mred\\u001b[0m'), util.toUSVString('x\\ud800y'), typeof debug, debug.enabled];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_util_helpers_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseUtil = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseUtil").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["value:4:{\"ok\":true}:%","custom",true,true,true,true,true,true,5,8,"red","x�y","function",false]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn util_parse_args_handles_long_short_multiple_negative_and_tokens() {
    use std::ffi::{CStr, CString};
    let dir = temp_registry("builtin_util_parse_args");
    fs::write(
            dir.join("index.js"),
            "var util = require('node:util'); module.exports = function () { var result = util.parseArgs({ args: ['-v', '--name=thaw', '--tag', 'one', '--tag=two', '--no-color', 'input.ts', '--', '-literal'], options: { verbose: { type: 'boolean', short: 'v' }, name: { type: 'string' }, tag: { type: 'string', multiple: true }, color: { type: 'boolean', default: true } }, allowNegative: true, allowPositionals: true, tokens: true }); var unknown; try { util.parseArgs({ args: ['--missing'], options: {} }); } catch (error) { unknown = error.code; } return [result.values.verbose, result.values.name, result.values.tag, result.values.color, result.positionals, result.tokens.map(function(token) { return token.kind; }), unknown, new util.TextEncoder().encode('ok').length, new util.TextDecoder().decode(Uint8Array.of(111, 107))]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_util_parse_args_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseUtilParseArgs = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let result_ptr = thaw_quickjs::thaw_js_call(
        CString::new("exerciseUtilParseArgs").unwrap().as_ptr(),
        CString::new("[]").unwrap().as_ptr(),
    );
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"thaw",["one","two"],false,["input.ts","-literal"],["option","option","option","option","option","positional","option-terminator","positional"],"ERR_PARSE_ARGS_UNKNOWN_OPTION",2,"ok"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn buffer_builtin_shares_the_global_buffer_implementation() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_buffer");
    fs::write(
            dir.join("index.js"),
            "var buffer = require('node:buffer');\n\
             module.exports = function () {\n\
             \x20 var value = buffer.Buffer.from('雪', 'utf8'); var invalid = buffer.Buffer.from([0xff]);\n\
             \x20 return [buffer.Buffer === globalThis.Buffer, value.toString('hex'), value.toString('base64'), buffer.byteLength('雪'), buffer.isUtf8(value), buffer.isUtf8(invalid), buffer.isAscii(buffer.Buffer.from('abc')), buffer.transcode(buffer.Buffer.from('hi'), 'utf8', 'utf16le').toString('hex')];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_buffer_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseBuffer = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseBuffer").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,"e99baa","6Zuq",3,true,false,true,"68006900"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn path_builtin_exposes_posix_and_win32_operations() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_path_operations");
    fs::write(
            dir.join("index.js"),
            "var path = require('node:path');\n\
             module.exports = function () {\n\
             \x20 var parsed = path.parse('/tmp/archive.tar.gz');\n\
             \x20 return [path.normalize('/a//b/../c/'), path.relative('/a/b', '/a/c/d'), path.extname('archive.tar.gz'), path.basename('archive.tar.gz', '.gz'), parsed, path.format(parsed), path.isAbsolute('/a'), path.posix === path, path.win32.normalize('C:\\\\a\\\\..\\\\b'), path.win32.isAbsolute('C:\\\\a'), path.delimiter, path.win32.delimiter];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_path_operations_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exercisePath = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exercisePath").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["/a/c/","../c/d",".gz","archive.tar",{"root":"/","dir":"/tmp","base":"archive.tar.gz","ext":".gz","name":"archive.tar"},"/tmp/archive.tar.gz",true,true,"C:\\b",true,":",";"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn legacy_builtin_aliases_and_domains_preserve_node_behavior() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_legacy_aliases_domain");
    fs::write(
            dir.join("index.js"),
            "var path = require('node:path'), posix = require('path/posix'), win32 = require('node:path/win32'), util = require('node:util'), sys = require('sys'), domain = require('node:domain'), EventEmitter = require('node:events'); module.exports = function () { var errors = [], values = [], active = false, d = domain.create(); d.on('error', function(error) { errors.push([error.message, error.domain === d, error.domainThrown]); }); d.run(function() { active = domain.active === d && process.domain === d; throw new Error('run'); }); var receiver = { value: 4, callback: d.bind(function(extra) { values.push(this.value + extra); throw new Error('bound'); }) }; receiver.callback(3); var intercepted = d.intercept(function(value) { values.push(value); }); intercepted(new Error('intercepted')); intercepted(null, 9); var emitter = new EventEmitter(); d.add(emitter); var added = emitter.domain === d && d.members[0] === emitter; d.remove(emitter); return [posix === path.posix, win32 === path.win32, sys === util, posix.join('a', 'b'), win32.join('C:\\\\a', 'b'), active, domain.active, process.domain, errors, values, added, emitter.domain, d.members.length]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_legacy_aliases_domain_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 8);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseLegacyBuiltins = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseLegacyBuiltins").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,true,"a/b","C:\\a\\b",true,null,null,[["run",true,true],["bound",true,true],["intercepted",true,true]],[7,9],true,null,0]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn internal_stream_aliases_and_trace_categories_share_state() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_stream_aliases_trace");
    fs::write(
            dir.join("index.js"),
            "var stream = require('node:stream'), Readable = require('_stream_readable'), Writable = require('node:_stream_writable'), Duplex = require('_stream_duplex'), Transform = require('_stream_transform'), PassThrough = require('_stream_passthrough'), StreamWrap = require('_stream_wrap'), trace = require('node:trace_events'); module.exports = function () { var first = trace.createTracing({ categories: ['node', 'v8', 'v8'] }), second = new trace.Tracing({ categories: ['v8', 'custom'] }), states = [first.enabled, trace.getEnabledCategories()]; first.enable(); first.enable(); states.push(first.enabled, trace.getEnabledCategories()); second.enable(); states.push(trace.getEnabledCategories()); first.disable(); states.push(first.enabled, trace.getEnabledCategories()); second.disable(); states.push(trace.getEnabledCategories()); return [Readable === stream.Readable, Writable === stream.Writable, Duplex === stream.Duplex, Transform === stream.Transform, PassThrough === stream.PassThrough, StreamWrap === stream.Duplex, first.categories, second.categories, states]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_stream_aliases_trace_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 9);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseTraceAliases = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseTraceAliases").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,true,true,true,true,["node","v8"],["v8","custom"],[false,"",true,"node,v8","custom,node,v8",false,"custom,v8",""]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn inspector_sessions_evaluate_through_callback_and_promise_protocols() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_inspector_sessions");
    fs::write(
            dir.join("index.js"),
            "var inspector = require('node:inspector'), promises = require('node:inspector/promises'); function post(session, method, params) { return new Promise(function(resolve) { session.post(method, params, function(error, result) { resolve(error ? { error: error.code } : result); }); }); } module.exports = async function () { var before = inspector.url(), disposable = inspector.open(9333, 'localhost'), opened = inspector.url(), callbackSession = new inspector.Session(); callbackSession.connect(); var evaluated = await post(callbackSession, 'Runtime.evaluate', { expression: '6 * 7', returnByValue: true }), exception = await post(callbackSession, 'Runtime.evaluate', { expression: 'throw new Error(\"boom\")' }), isolate = await post(callbackSession, 'Runtime.getIsolateId'), schema = await post(callbackSession, 'Schema.getDomains'), unsupported = await post(callbackSession, 'Network.enable'); callbackSession.disconnect(); var disconnected = await post(callbackSession, 'Runtime.enable'), promiseSession = new promises.Session(); promiseSession.connectToMainThread(); var object = await promiseSession.post('Runtime.evaluate', { expression: '({ answer: 42 })', returnByValue: true }); await promiseSession.post('Debugger.enable'); promiseSession.disconnect(); disposable.dispose(); return [before, opened, inspector.url(), evaluated.result, exception.exceptionDetails.text, isolate.id, schema.domains.map(function(value) { return value.name; }), unsupported.error, disconnected.error, object.result.value, inspector.console === console]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_inspector_sessions_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseInspector = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseInspector").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[null,"ws://localhost:9333/thaw",null,{"type":"number","value":42},"boom","thaw-quickjs-main",["Runtime","Debugger","Profiler","HeapProfiler","Schema"],"ERR_INSPECTOR_COMMAND","ERR_INSPECTOR_NOT_CONNECTED",{"answer":42},true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn repl_server_evaluates_input_and_drives_commands() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_repl_server");
    fs::write(
            dir.join("index.js"),
            "var repl = require('node:repl'); module.exports = function () { var output = { text: '', write: function(value) { this.text += value; } }, events = [], server = repl.start({ prompt: 'thaw> ', output: output, historySize: 2, replMode: repl.REPL_MODE_STRICT, writer: function(value) { return '<' + String(value) + '>'; } }); server.on('result', function(value) { events.push(value); }); server.on('exit', function() { events.push('exit'); }); server.defineCommand('double', { help: 'double a value', action: function(value) { this.write(String(Number(value) * 2)); } }); server.write('20 + 22'); server.write('.double 5'); server.write('1'); server.write('2'); var history = server.history.slice(); server.setPrompt('next> '); server.displayPrompt(); var setup = false; server.setupHistory('/tmp/history', function(error) { setup = !error; }); server.write('.exit'); return Promise.resolve().then(function() { return [server instanceof repl.REPLServer, server.closed, server.last, server.context._, history, events, setup, output.text, typeof repl.writer({ answer: 42 }), repl._builtinLibs.length, repl.REPL_MODE_SLOPPY !== repl.REPL_MODE_STRICT]; }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_repl_server_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseRepl = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseRepl").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,2,2,["2","1"],[42,10,1,2,"exit"],true,"thaw> <42>\nthaw> <10>\nthaw> <1>\nthaw> <2>\nthaw> next> ","string",0,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn cluster_primary_tracks_worker_lifecycle_and_messages() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_cluster_primary");
    fs::write(
            dir.join("index.js"),
            "var cluster = require('node:cluster'); module.exports = async function () { var events = []; cluster.on('setup', function(settings) { events.push('setup:' + settings.exec); }); cluster.on('fork', function(worker) { events.push('fork:' + worker.id); }); cluster.on('online', function(worker) { events.push('online:' + worker.id); }); cluster.on('message', function(worker, message) { events.push('cluster-message:' + message.value); }); cluster.on('disconnect', function(worker) { events.push('cluster-disconnect:' + worker.id); }); cluster.on('exit', function(worker, code, signal) { events.push('cluster-exit:' + signal); }); cluster.setupPrimary({ exec: 'service.js', args: ['--serve'] }); cluster.schedulingPolicy = cluster.SCHED_NONE; var worker = cluster.fork({ PORT: '8080' }); worker.on('online', function() { events.push('worker-online'); }); worker.on('message', function(message) { events.push('worker-message:' + message.value); }); worker.on('disconnect', function() { events.push('worker-disconnect'); }); worker.on('exit', function(code, signal) { events.push('worker-exit:' + signal); }); worker.send({ value: 42 }, function(error) { events.push(error ? error.code : 'sent'); }); await new Promise(function(resolve) { cluster.disconnect(function() { events.push('all-disconnected'); resolve(); }); }); var connected = worker.isConnected(), exitedAfterDisconnect = worker.exitedAfterDisconnect; worker.kill('SIGKILL'); await Promise.resolve(); return [cluster.isPrimary, cluster.isMaster, cluster.isWorker, cluster.settings.exec, cluster.schedulingPolicy, worker.environment.PORT, connected, exitedAfterDisconnect, worker.isDead(), Object.keys(cluster.workers).length, events]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_cluster_primary_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 4);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseCluster = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseCluster").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,false,"service.js",1,"8080",false,true,true,0,["setup:service.js","fork:1","worker-online","online:1","worker-message:42","cluster-message:42","sent","worker-disconnect","cluster-disconnect:1","all-disconnected","worker-exit:SIGKILL","cluster-exit:SIGKILL"]]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn internal_http_and_tls_modules_share_public_implementations() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_internal_http_tls");
    fs::write(
            dir.join("index.js"),
            "var http = require('node:http'), tls = require('node:tls'), agent = require('_http_agent'), client = require('node:_http_client'), common = require('_http_common'), incoming = require('_http_incoming'), outgoing = require('_http_outgoing'), server = require('_http_server'), tlsCommon = require('_tls_common'), tlsWrap = require('_tls_wrap'); module.exports = function () { var context = tlsCommon.createSecureContext({ minVersion: 'TLSv1.2' }); context.setKey('key'); context.setCert('cert'); context.addCACert('ca'); var parser = common.parsers.alloc(); parser.initialize(common.HTTPParser.RESPONSE); common.parsers.free(parser); return [agent.Agent === http.Agent, agent.globalAgent === http.globalAgent, client.ClientRequest === http.ClientRequest, incoming.IncomingMessage === http.IncomingMessage, outgoing.OutgoingMessage === http.ServerResponse, server.Server === http.Server, server.ServerResponse === http.ServerResponse, tlsWrap.TLSSocket === tls.TLSSocket, tlsCommon.SecureContext === tls.SecureContext, context instanceof tls.SecureContext, context.options, common.methods === http.METHODS, common._checkIsHttpToken('x-test'), common._checkIsHttpToken('bad header'), common._checkInvalidHeaderChar('ok\\nno'), common.kLenientAll, common.parsers.list.length]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_internal_http_tls_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 13);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseInternalHttpTls = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseInternalHttpTls").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,true,true,true,true,true,true,true,true,{"minVersion":"TLSv1.2","key":"key","cert":"cert","ca":["ca"]},true,true,false,true,1023,1]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http2_h2c_sessions_exchange_real_frames_over_tcp() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http2_h2c");
    fs::write(
            dir.join("index.js"),
            "var http2 = require('node:http2'); module.exports = function () { return new Promise(function(resolve, reject) { var large = 'x'.repeat(20000), server = http2.createServer(); server.on('error', reject); server.on('stream', function(stream, headers) { stream.respond({ ':status': 200, 'content-type': 'text/plain', 'x-path': headers[':path'], 'x-return': headers['x-large'] }); stream.end('hello ' + headers['x-name']); }); server.listen(0, '127.0.0.1', function() { var address = server.address(), session = http2.connect('http://127.0.0.1:' + address.port); session.on('error', reject); session.on('connect', function() { var ping = Buffer.from('12345678'), request = session.request({ ':path': '/demo', 'x-name': 'thaw', 'x-large': large }), chunks = [], response; request.on('response', function(headers) { response = headers; }); request.on('data', function(chunk) { chunks.push(Buffer.from(chunk)); }); request.on('end', function() { session.ping(ping, function(error, duration, payload) { if (error) return reject(error); var packed = http2.getPackedSettings({ enablePush: false, maxConcurrentStreams: 12 }), unpacked = http2.getUnpackedSettings(packed); session.close(); server.close(function() { resolve([session instanceof http2.ClientHttp2Session, request instanceof http2.ClientHttp2Stream, response[':status'], response['content-type'], response['x-path'], response['x-return'].length, Buffer.concat(chunks).toString(), payload.toString(), duration >= 0, unpacked.enablePush, unpacked.maxConcurrentStreams, http2.constants.NGHTTP2_NO_ERROR, typeof http2.sensitiveHeaders]); }); }); }); request.end(); }); }); }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http2_h2c_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttp2 = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttp2").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[true,true,"200","text/plain","/demo",20000,"hello thaw","12345678",true,0,12,0,"symbol"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http2_flow_control_resumes_large_bidirectional_bodies() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http2_flow_control");
    fs::write(
            dir.join("index.js"),
            "var http2 = require('node:http2'); module.exports = function () { return new Promise(function(resolve, reject) { var requestBody = 'q'.repeat(100000), responseBody = 'z'.repeat(120000), server = http2.createServer(); server.on('error', reject); server.on('stream', function(stream) { var received = 0; stream.on('data', function(chunk) { received += chunk.length; }); stream.on('end', function() { stream.respond({ ':status': 201, 'x-received': String(received) }); stream.end(responseBody); }); }); server.listen(0, '127.0.0.1', function() { var session = http2.connect('http://127.0.0.1:' + server.address().port); session.on('error', reject); session.on('connect', function() { var request = session.request({ ':method': 'POST', ':path': '/upload' }), chunks = [], response, immediate = request.write(requestBody); request.on('response', function(headers) { response = headers; }); request.on('data', function(chunk) { chunks.push(Buffer.from(chunk)); }); request.on('end', function() { var body = Buffer.concat(chunks); session.close(); server.close(function() { resolve([immediate, request.writableEnded, response[':status'], response['x-received'], body.length, body[0], body[body.length - 1]]); }); }); request.end(); }); }); }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http2_flow_control_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttp2Flow = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttp2Flow").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[false,true,"201","100000",120000,122,122]"#);
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http2_hpack_reuses_dynamic_entries_across_streams() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_http2_hpack_dynamic");
    fs::write(
            dir.join("index.js"),
            "var http2 = require('node:http2'); module.exports = function () { return new Promise(function(resolve, reject) { var serverSession, server = http2.createServer(); server.on('session', function(session) { serverSession = session; }); server.on('stream', function(stream, headers) { stream.respond({ ':status': 200, 'x-repeated-response': 'compress-me-compress-me' }); stream.end(headers['x-repeated-request']); }); server.listen(0, '127.0.0.1', function() { var session = http2.connect('http://127.0.0.1:' + server.address().port); session.on('error', reject); session.on('connect', async function() { function run(path) { return new Promise(function(done) { var request = session.request({ ':path': path, 'x-repeated-request': 'www.example.com' }), chunks = []; request.on('data', function(chunk) { chunks.push(Buffer.from(chunk)); }); request.on('end', function() { done(Buffer.concat(chunks).toString()); }); request.end(); }); } var first = await run('/first'), firstEncoderSize = session._encoderTable.length, second = await run('/second'); var clientEntry = session._encoderTable.some(function(entry) { return entry[0] === 'x-repeated-request' && entry[1] === 'www.example.com'; }), serverEntry = serverSession._decoderTable.some(function(entry) { return entry[0] === 'x-repeated-request' && entry[1] === 'www.example.com'; }), responseEntry = session._decoderTable.some(function(entry) { return entry[0] === 'x-repeated-response'; }); session.close(); server.close(function() { resolve([first, second, firstEncoderSize > 0, clientEntry, serverEntry, responseEntry]); }); }); }); }); };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http2_hpack_dynamic_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttp2Hpack = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttp2Hpack").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["www.example.com","www.example.com",true,true,true,true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn http2_tls_sessions_negotiate_h2_with_alpn() {
    use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
    use rustls::{ServerConfig, ServerConnection, StreamOwned};
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::process::Command;
    use std::sync::Arc;

    let certificate_dir = temp_registry("builtin_http2_tls_certificate");
    let key_path = certificate_dir.join("key.pem");
    let cert_path = certificate_dir.join("cert.pem");
    let key_der_path = certificate_dir.join("key.der");
    let cert_der_path = certificate_dir.join("cert.der");
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
        .arg(&key_path)
        .arg("-out")
        .arg(&cert_path)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args(["x509", "-in"])
        .arg(&cert_path)
        .args(["-outform", "DER", "-out"])
        .arg(&cert_der_path)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args(["pkcs8", "-topk8", "-nocrypt", "-in"])
        .arg(&key_path)
        .args(["-outform", "DER", "-out"])
        .arg(&key_der_path)
        .output()
        .unwrap()
        .status
        .success());
    let certificate = fs::read_to_string(&cert_path).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut tls_config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(fs::read(&cert_der_path).unwrap())],
            PrivatePkcs8KeyDer::from(fs::read(&key_der_path).unwrap()).into(),
        )
        .unwrap();
    tls_config.alpn_protocols = vec![b"h2".to_vec()];
    let tls_config = Arc::new(tls_config);
    let peer = std::thread::spawn(move || {
        let (socket, _) = listener.accept().unwrap();
        let connection = ServerConnection::new(tls_config).unwrap();
        let mut stream = StreamOwned::new(connection, socket);
        let mut preface = [0u8; 24];
        stream.read_exact(&mut preface).unwrap();
        assert_eq!(&preface, b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");
        loop {
            let mut header = [0u8; 9];
            stream.read_exact(&mut header).unwrap();
            let length =
                ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
            let mut payload = vec![0u8; length];
            stream.read_exact(&mut payload).unwrap();
            if header[3] == 1 {
                let body = b"encrypted";
                let response = [
                    vec![0, 0, 0, 4, 0, 0, 0, 0, 0],
                    vec![0, 0, 1, 1, 4, 0, 0, 0, 1, 0x88],
                    vec![0, 0, body.len() as u8, 0, 1, 0, 0, 0, 1],
                    body.to_vec(),
                ]
                .concat();
                stream.write_all(&response).unwrap();
                stream.flush().unwrap();
                std::thread::sleep(std::time::Duration::from_millis(250));
                break;
            }
        }
    });
    let dir = temp_registry("builtin_http2_tls");
    fs::write(
            dir.join("index.js"),
            format!(
                "var http2 = require('node:http2'), certificate = {}; module.exports = function () {{ return new Promise(function(resolve, reject) {{ var secureServer = http2.createSecureServer({{}}), session = http2.connect('https://127.0.0.1:{}', {{ ca: certificate }}); session.on('error', reject); session.on('connect', function() {{ var request = session.request({{ ':path': '/secure' }}), chunks = [], response; request.on('response', function(headers) {{ response = headers; }}); request.on('data', function(chunk) {{ chunks.push(Buffer.from(chunk)); }}); request.on('end', function() {{ session.close(); resolve([secureServer instanceof http2.Http2SecureServer, session.encrypted, session.alpnProtocol, session.socket.alpnProtocol, response[':status'], Buffer.concat(chunks).toString()]); }}); request.end(); }}); }}); }};",
                serde_json::to_string(&certificate).unwrap(),
                port
            ),
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http2_tls_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttp2Tls = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttp2Tls").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,true,"h2","h2","200","encrypted"]"#);
    peer.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    let _ = fs::remove_dir_all(&certificate_dir);
}

#[test]
fn http2_secure_server_accepts_an_alpn_h2_peer() {
    use rustls::pki_types::{CertificateDer, ServerName};
    use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
    use std::ffi::{CStr, CString};
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::process::Command;
    use std::sync::Arc;
    use std::time::Duration;

    let certificate_dir = temp_registry("builtin_http2_secure_server_certificate");
    let key_path = certificate_dir.join("key.pem");
    let cert_path = certificate_dir.join("cert.pem");
    let cert_der_path = certificate_dir.join("cert.der");
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
        .arg(&key_path)
        .arg("-out")
        .arg(&cert_path)
        .output()
        .unwrap()
        .status
        .success());
    assert!(Command::new("openssl")
        .args(["x509", "-in"])
        .arg(&cert_path)
        .args(["-outform", "DER", "-out"])
        .arg(&cert_der_path)
        .output()
        .unwrap()
        .status
        .success());
    let certificate = fs::read_to_string(&cert_path).unwrap();
    let key = fs::read_to_string(&key_path).unwrap();
    let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = reservation.local_addr().unwrap().port();
    drop(reservation);
    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from(fs::read(&cert_der_path).unwrap()))
        .unwrap();
    let mut client_config = ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    client_config.alpn_protocols = vec![b"h2".to_vec()];
    let client_config = Arc::new(client_config);
    let peer = std::thread::spawn(move || {
        let socket = (0..100)
            .find_map(|_| match TcpStream::connect(("127.0.0.1", port)) {
                Ok(socket) => Some(socket),
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(10));
                    None
                }
            })
            .expect("HTTP/2 secure server did not start");
        let connection = ClientConnection::new(
            client_config,
            ServerName::try_from("127.0.0.1".to_string()).unwrap(),
        )
        .unwrap();
        let mut stream = StreamOwned::new(connection, socket);
        let request = [
            b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n".to_vec(),
            vec![0, 0, 0, 4, 0, 0, 0, 0, 0],
            vec![0, 0, 3, 1, 5, 0, 0, 0, 1, 0x82, 0x84, 0x87],
        ]
        .concat();
        stream.write_all(&request).unwrap();
        stream.flush().unwrap();
        let mut body = Vec::new();
        loop {
            let mut header = [0u8; 9];
            stream.read_exact(&mut header).unwrap();
            let length =
                ((header[0] as usize) << 16) | ((header[1] as usize) << 8) | header[2] as usize;
            let mut payload = vec![0u8; length];
            stream.read_exact(&mut payload).unwrap();
            if header[3] == 0 {
                body.extend(payload);
                if header[4] & 1 != 0 {
                    break;
                }
            }
        }
        (
            stream
                .conn
                .alpn_protocol()
                .map(|value| value.to_vec())
                .unwrap_or_default(),
            body,
        )
    });
    let dir = temp_registry("builtin_http2_secure_server");
    fs::write(
            dir.join("index.js"),
            format!(
                "var http2 = require('node:http2'), certificate = {}, key = {}; module.exports = function () {{ return new Promise(function(resolve, reject) {{ var sessionState, server = http2.createSecureServer({{ cert: certificate, key: key }}); server.on('error', reject); server.on('sessionError', reject); server.on('session', function(session) {{ sessionState = [session.encrypted, session.alpnProtocol, session.socket.alpnProtocol]; }}); server.on('stream', function(stream, headers) {{ stream.on('close', function() {{ server.close(function() {{ resolve([server instanceof http2.Http2SecureServer, sessionState, headers[':scheme']]); }}); }}); stream.respond({{ ':status': 202 }}); stream.end('served'); }}); server.listen({}, '127.0.0.1'); }}); }};",
                serde_json::to_string(&certificate).unwrap(),
                serde_json::to_string(&key).unwrap(),
                port
            ),
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_http2_secure_server_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseHttp2SecureServer = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseHttp2SecureServer").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(result, r#"[true,[true,"h2","h2"],"https"]"#);
    let (alpn, body) = peer.join().unwrap();
    assert_eq!(alpn, b"h2");
    assert_eq!(body, b"served");
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    let _ = fs::remove_dir_all(&certificate_dir);
}

#[test]
fn node_test_runs_suites_hooks_mocks_and_reporters() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_node_test");
    fs::write(
            dir.join("index.js"),
            "var test = require('node:test'), reporters = require('node:test/reporters'), assert = require('node:assert'); module.exports = async function () { var events = [], object = { add: function(value) { return value + 1; } }; var suite = await test.describe('math', function() { test.before(function() { events.push('before'); }); test.after(function() { events.push('after'); }); test.beforeEach(function() { events.push('beforeEach'); }); test.afterEach(function() { events.push('afterEach'); }); test.it('adds', async function(t) { events.push('test'); var mocked = t.mock.method(object, 'add', function(value) { return value * 2; }); assert.strictEqual(object.add(4), 8); assert.strictEqual(mocked.mock.callCount(), 1); await Promise.resolve(); }); test.skip('skipped', function() { throw new Error('must not run'); }); test.todo('later', function() { throw new Error('must not run'); }); }); var restored = object.add(4), raw = [], stream = test.run(); for await (var event of stream) raw.push([event.type, event.name, event.status]); var text = '', formatted = test.run(); for await (var line of reporters.spec(formatted)) text += line; return [suite.status, events, restored, raw, text.includes('✔ math > adds'), text.includes('- math > skipped'), test === test.test, test.it === test, typeof test.mock.fn(function() {})]; };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_node_test_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 5);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNodeTest = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNodeTest").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["passed",["before","beforeEach","test","afterEach","after"],5,[["test","adds","passed"],["test","skipped","skipped"],["test","later","todo"],["suite","math","passed"]],true,true,true,true,"function"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn node_wasi_runs_preview1_commands_and_reactors() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_node_wasi");
    let wasi_source = r#"var WASI = require('node:wasi').WASI;
               module.exports = function () {
                 var commandSource = new TextEncoder().encode(`(module
                   (import "wasi_snapshot_preview1" "args_sizes_get" (func $args_sizes_get (param i32 i32) (result i32)))
                   (import "wasi_snapshot_preview1" "environ_sizes_get" (func $environ_sizes_get (param i32 i32) (result i32)))
                   (import "wasi_snapshot_preview1" "proc_exit" (func $proc_exit (param i32)))
                   (memory (export "memory") 1)
                   (func (export "_start")
                     i32.const 0 i32.const 4 call $args_sizes_get drop
                     i32.const 8 i32.const 12 call $environ_sizes_get drop
                     i32.const 7 call $proc_exit))`);
                 var wasi = new WASI({ version: 'preview1', args: ['alpha', 'beta'], env: { A: '1', B: 'two' }, returnOnExit: true });
                 var command = new WebAssembly.Instance(new WebAssembly.Module(commandSource), wasi.getImportObject());
                 var exit = wasi.start(command), words = new Uint32Array(command.exports.memory.buffer, 0, 4), repeated;
                 try { wasi.start(command); } catch (error) { repeated = error.code; }
                 var reactorSource = new TextEncoder().encode(`(module (memory (export "memory") 1) (func (export "_initialize")))`);
                 var reactorWasi = new WASI({ version: 'preview1' });
                 var reactor = new WebAssembly.Instance(new WebAssembly.Module(reactorSource), reactorWasi.getImportObject());
                 var initialized = reactorWasi.initialize(reactor), invalid = false;
                 try { new WASI({ version: 'preview2' }); } catch (error) { invalid = error.code === 'ERR_INVALID_ARG_VALUE'; }
                 var preopenSource = new TextEncoder().encode(`(module
                   (import "wasi_snapshot_preview1" "fd_prestat_get" (func $get (param i32 i32) (result i32)))
                   (import "wasi_snapshot_preview1" "fd_prestat_dir_name" (func $name (param i32 i32 i32) (result i32)))
                   (memory (export "memory") 1)
                   (func (export "_start") i32.const 3 i32.const 0 call $get drop i32.const 3 i32.const 16 i32.const 8 call $name drop))`);
                 var preopenWasi = new WASI({ version: 'preview1', preopens: { '/sandbox': __HOST_PATH__ }, returnOnExit: true });
                 var preopen = new WebAssembly.Instance(new WebAssembly.Module(preopenSource), preopenWasi.getImportObject()); preopenWasi.start(preopen);
                 var preopenLength = new Uint32Array(preopen.exports.memory.buffer, 4, 1)[0], preopenName = new TextDecoder().decode(new Uint8Array(preopen.exports.memory.buffer, 16, preopenLength));
                 return [exit, Array.from(words), repeated, initialized, invalid, wasi.wasiImport === wasi.getImportObject().wasi_snapshot_preview1, preopenLength, preopenName];
               };"#
        .replace(
            "__HOST_PATH__",
            &serde_json::to_string(dir.to_str().unwrap()).unwrap(),
        );
    fs::write(dir.join("index.js"), wasi_source).unwrap();
    let empty_node_modules = temp_registry("builtin_node_wasi_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!("globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNodeWasi = module.exports;");
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNodeWasi").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[7,[2,11,2,10],"ERR_WASI_ALREADY_STARTED",null,true,true,8,"/sandbox"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn string_decoder_preserves_multibyte_boundaries() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_string_decoder");
    fs::write(
            dir.join("index.js"),
            "var StringDecoder = require('node:string_decoder').StringDecoder;\n\
             module.exports = function () {\n\
             \x20 var utf8 = new StringDecoder('utf8'); var snow = Buffer.from('雪'); var utf8Parts = [utf8.write(snow.subarray(0, 1)), utf8.write(snow.subarray(1, 2)), utf8.write(snow.subarray(2)), utf8.end()];\n\
             \x20 var utf16 = new StringDecoder('utf16le'); var wide = Buffer.from('A雪', 'utf16le'); var utf16Parts = [utf16.write(wide.subarray(0, 3)), utf16.end(wide.subarray(3))];\n\
             \x20 var base64 = new StringDecoder('base64'); var hello = Buffer.from('hello'); var encoded = base64.write(hello.subarray(0, 2)) + base64.write(hello.subarray(2)) + base64.end();\n\
             \x20 var incomplete = new StringDecoder(); var replacement = incomplete.end(Buffer.from([0xe9]));\n\
             \x20 return [utf8Parts, utf16Parts, encoded, replacement, utf8.lastNeed, utf8.encoding];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_string_decoder_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 2);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseStringDecoder = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseStringDecoder").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"[["","","雪",""],["A","雪"],"aGVsbG8=","�",0,"utf8"]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}

#[test]
fn timer_modules_share_the_runtime_queue_and_support_abort() {
    use std::ffi::{CStr, CString};

    let dir = temp_registry("builtin_timers");
    fs::write(
            dir.join("index.js"),
            "var timers = require('node:timers'); var promises = require('node:timers/promises');\n\
             module.exports = async function () {\n\
             \x20 var immediate = await new Promise(function(resolve) { timers.setImmediate(resolve, 'immediate'); });\n\
             \x20 var delayed = await promises.setTimeout(0, 'delayed'); await promises.scheduler.yield();\n\
             \x20 var controller = new AbortController(); controller.abort('cancelled'); var reason; try { await promises.setTimeout(1, 'wrong', { signal: controller.signal }); } catch (error) { reason = error; }\n\
             \x20 var interval = promises.setInterval(0, 'tick'); var first = await interval.next(); var second = await interval.next(); var ended = await interval.return();\n\
             \x20 return [immediate, delayed, reason, first.value, first.done, second.value, ended.done];\n\
             };",
        )
        .unwrap();
    let empty_node_modules = temp_registry("builtin_timers_node_modules");
    let (bundle, _, file_count, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    assert_eq!(file_count, 3);
    let script = format!(
            "globalThis.module = {{ exports: {{}} }};\n\
             globalThis.exports = globalThis.module.exports;\n\
             globalThis.require = function(name) {{ throw new Error(\"require('\" + name + \"') is not supported\"); }};\n\
             {bundle}\n\
             globalThis.exerciseTimers = module.exports;\n"
        );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let func = CString::new("exerciseTimers").unwrap();
    let args = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(func.as_ptr(), args.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }.to_string_lossy();
    assert_eq!(
        result,
        r#"["immediate","delayed","cancelled","tick",false,"tick",true]"#
    );
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
}
