// Contract tests for the Node API surface fixes (audit2 group E). Expected values are real Node 22 output.

fn run_node_api_js(tag: &str, body: &str) -> String {
    use std::ffi::{CStr, CString};
    let dir = temp_registry(tag);
    fs::write(
        dir.join("index.js"),
        format!("module.exports = async function () {{ {body} }};"),
    )
    .unwrap();
    let empty_node_modules = temp_registry(&format!("{tag}_node_modules"));
    let (bundle, _, _, _) =
        bundle_commonjs_package(&empty_node_modules, "pkg", &dir, "index.js").unwrap();
    let script = format!(
        "globalThis.module = {{ exports: {{}} }}; globalThis.exports = globalThis.module.exports; globalThis.require = function(name) {{ throw new Error(name); }}; {bundle} globalThis.exerciseNodeApi = module.exports;"
    );
    let source = CString::new(script).unwrap();
    assert_eq!(thaw_quickjs::thaw_js_load(source.as_ptr()), 1);
    let function = CString::new("exerciseNodeApi").unwrap();
    let arguments = CString::new("[]").unwrap();
    let result_ptr = thaw_quickjs::thaw_js_call(function.as_ptr(), arguments.as_ptr());
    let result = unsafe { CStr::from_ptr(result_ptr) }
        .to_string_lossy()
        .into_owned();
    let _ = fs::remove_dir_all(&dir);
    let _ = fs::remove_dir_all(&empty_node_modules);
    result
}

#[test]
fn assert_deep_equal_is_loose_and_reports_loose_message() {
    let body = r####"var assert = require('node:assert'); var r = [];
try { assert.deepEqual({ a: 1 }, { a: '1' }); r.push('loose-ok'); } catch (e) { r.push(e.message); }
try { assert.deepEqual([1, 2], [1, 2, 3]); } catch (e) { r.push(e.operator, e.message); }
try { assert.notDeepEqual({ a: 1 }, { a: '1' }); } catch (e) { r.push(e.operator); }
try { assert.deepStrictEqual({ a: 1 }, { a: '1' }); r.push('no'); } catch (e) { r.push('strict-throws'); }
assert.deepEqual(NaN, NaN); assert.deepEqual(new Date(0), new Date(0)); assert.deepEqual(null, undefined);
return r;"####;
    let expected = r####"["loose-ok","deepEqual","Expected values to be loosely deep-equal:\n\n[\n  1,\n  2\n]\n\nshould loosely deep-equal\n\n[\n  1,\n  2,\n  3\n]","notDeepEqual","strict-throws"]"####;
    assert_eq!(run_node_api_js("node_api_assert_loose", body), expected);
}

#[test]
fn assert_rejects_does_not_reject_and_assertion_error_message() {
    let body = r####"var assert = require('node:assert'); var r = [];
return assert.rejects(Promise.reject(new Error('r')), { message: 'r' }).then(function () { return assert.rejects(Promise.resolve(1)).then(function () { r.push('no'); }, function (e) { r.push(e.code, e.message); }); }).then(function () { return assert.doesNotReject(Promise.reject(new Error('dnr'))).then(function () { r.push('no'); }, function (e) { r.push(e.message); }); }).then(function () { return assert.doesNotReject(Promise.resolve(1)); }).then(function () { r.push(new assert.AssertionError({ actual: 1, expected: 2, operator: 'strictEqual' }).message); return r; });"####;
    let expected = r####"["ERR_ASSERTION","Missing expected rejection.","Got unwanted rejection.\nActual message: \"dnr\"","Expected values to be strictly equal:\n\n1 !== 2\n"]"####;
    assert_eq!(run_node_api_js("node_api_assert_rejects", body), expected);
}

#[test]
fn crypto_ripemd160_hash_hmac_pbkdf2_and_module_get_random_values() {
    let body = r####"var crypto = require('node:crypto');
return [crypto.createHash('ripemd160').update('abc').digest('hex'), crypto.createHash('rmd160').update('').digest('hex'), crypto.createHmac('ripemd160', 'k').update('abc').digest('hex'), crypto.getRandomValues(new Uint8Array(3)).length, crypto.pbkdf2Sync('p', 's', 2, 8, 'ripemd160').toString('hex')];"####;
    let expected = r####"["8eb208f7e05d987a9b044a8e98c6b087f15a0bfc","9c1185a5c5e9fc54612808977ee8f548b2258d31","760bba2de2cd21e5a26229f0083e9e68deb4f2ea",3,"332c478ecdc03e9f"]"####;
    assert_eq!(
        run_node_api_js("node_api_crypto_ripemd_random", body),
        expected
    );
}

#[test]
fn crypto_aes_ctr_gcm_ecb_cbc_sizes_and_error_shapes() {
    let body = r####"var crypto = require('node:crypto'); var r = [];
var ctr = crypto.createCipheriv('aes-128-ctr', Buffer.alloc(16), Buffer.alloc(16)); r.push(Buffer.concat([ctr.update('hel'), ctr.update('lo world, longer than a block'), ctr.final()]).toString('hex'));
var ctrd = crypto.createDecipheriv('aes-256-ctr', Buffer.alloc(32, 1), Buffer.alloc(16, 2)); var ctre = crypto.createCipheriv('aes-256-ctr', Buffer.alloc(32, 1), Buffer.alloc(16, 2)); r.push(ctrd.update(ctre.update('round trip'), undefined, 'utf8'));
var g = crypto.createCipheriv('aes-128-gcm', Buffer.alloc(16, 3), Buffer.alloc(12, 4)); g.setAAD(Buffer.from('aad')); var ge = Buffer.concat([g.update('gcm text'), g.final()]); var tag = g.getAuthTag(); r.push(ge.toString('hex'), tag.toString('hex'));
var gd = crypto.createDecipheriv('aes-128-gcm', Buffer.alloc(16, 3), Buffer.alloc(12, 4)); gd.setAAD(Buffer.from('aad')); gd.setAuthTag(tag); r.push(Buffer.concat([gd.update(ge), gd.final()]).toString());
var bad = crypto.createDecipheriv('aes-128-gcm', Buffer.alloc(16, 3), Buffer.alloc(12, 4)); bad.setAAD(Buffer.from('zzz')); bad.setAuthTag(tag); bad.update(ge); try { bad.final(); r.push('no'); } catch (e) { r.push(e.message); }
var g256 = crypto.createCipheriv('aes-256-gcm', Buffer.alloc(32, 5), Buffer.alloc(16, 6)); r.push(Buffer.concat([g256.update('long iv'), g256.final()]).toString('hex'), g256.getAuthTag().toString('hex'));
var e = crypto.createCipheriv('aes-128-ecb', Buffer.alloc(16), null); r.push(Buffer.concat([e.update('x'), e.final()]).toString('hex'));
var c192 = crypto.createCipheriv('aes-192-cbc', Buffer.alloc(24, 7), Buffer.alloc(16, 8)); r.push(Buffer.concat([c192.update('cbc 192 data'), c192.final()]).toString('hex'));
var np = crypto.createCipheriv('aes-128-cbc', Buffer.alloc(16), Buffer.alloc(16)); np.setAutoPadding(false); r.push(Buffer.concat([np.update(Buffer.alloc(16, 9)), np.final()]).toString('hex'));
[['aes-256-cbc', 5, 16], ['aes-256-cbc', 32, 5], ['aes-128-ecb', 16, 5], ['nope-cipher', 16, 16]].forEach(function (a) { try { crypto.createCipheriv(a[0], Buffer.alloc(a[1]), Buffer.alloc(a[2])); } catch (x) { r.push(x.name + ' ' + x.code + ' ' + x.message); } });
return r;"####;
    let expected = r####"["0e8c27b880aa5b54fa209e75ea5844403f878eee8e16510f161e3d35c8882631","round trip","8666727c54d64fe6","878c8a276cbcb4a5887d90aac15631d1","gcm text","Unsupported state or unable to authenticate data","d892291a79ebe1","6405e32511afe0cc82710c9e093a6fc3","1dd733a04c633c194fdeacf97c59d237","8fa92307f4ebb7108eaf73258b6fa092","ed66f4f85ca4c7d2955de53f47699293","RangeError ERR_CRYPTO_INVALID_KEYLEN Invalid key length","TypeError ERR_CRYPTO_INVALID_IV Invalid initialization vector","TypeError ERR_CRYPTO_INVALID_IV Invalid initialization vector","Error ERR_CRYPTO_UNKNOWN_CIPHER Unknown cipher"]"####;
    assert_eq!(run_node_api_js("node_api_crypto_modes", body), expected);
}

#[test]
fn url_format_adds_scheme_separators_and_parse_query_is_null_prototype_object() {
    let body = r####"var url = require('node:url'); var r = [];
r.push(url.format({ protocol: 'https', hostname: 'a.com', pathname: '/x', query: { a: '1' } }), url.format({ protocol: 'http:', host: 'a.com:80', search: '?q=1', hash: '#h' }), url.format({ protocol: 'https:', hostname: 'a.com', pathname: 'x', search: 'q=1', hash: 'h' }), url.format({ protocol: 'mailto:', pathname: 'a@b.c' }));
var p = url.parse('/p/a?x=1&x=2&y=3', true); r.push(Object.getPrototypeOf(p.query) === null, JSON.stringify(p.query), p.pathname, p.search, url.parse('/p?x=1').query, JSON.stringify(url.parse('/p', true).query));
r.push(url.resolve('http://a.com/b/c', '../d'), url.domainToASCII('münchen.de'), url.domainToUnicode('xn--mnchen-3ya.de'));
return r;"####;
    let expected = r####"["https://a.com/x?a=1","http://a.com:80?q=1#h","https://a.com/x?q=1#h","mailto:a@b.c",true,"{\"x\":[\"1\",\"2\"],\"y\":\"3\"}","/p/a","?x=1&x=2&y=3","x=1","{}","http://a.com/d","xn--mnchen-3ya.de","münchen.de"]"####;
    assert_eq!(run_node_api_js("node_api_url_format_parse", body), expected);
}

#[test]
fn zlib_crc32_options_and_constants() {
    let body = r####"var zlib = require('node:zlib');
var s = Buffer.from('hello hello hello hello');
return [zlib.crc32('abc'), zlib.crc32('def', zlib.crc32('abc')), zlib.crc32(Buffer.from('abcdef')), zlib.deflateSync(s, { level: 0 }).length, zlib.inflateSync(zlib.deflateSync(s, { level: 9 })).toString(), zlib.constants.Z_BEST_COMPRESSION, zlib.constants.Z_DEFAULT_COMPRESSION];"####;
    let expected = r####"[891568578,1267612143,1267612143,34,"hello hello hello hello",9,-1]"####;
    assert_eq!(run_node_api_js("node_api_zlib_crc_options", body), expected);
}

#[test]
fn buffer_module_exports_blob_atob_btoa() {
    let body = r####"var b = require('node:buffer'); var r = [b.btoa('hello'), b.atob('aGVsbG8='), typeof b.Blob, b.Blob === Blob];
return new b.Blob(['hello ', 'world']).text().then(function (t) { r.push(t); return r; });"####;
    let expected = r####"["aGVsbG8=","hello","function",true,"hello world"]"####;
    assert_eq!(run_node_api_js("node_api_buffer_exports", body), expected);
}
