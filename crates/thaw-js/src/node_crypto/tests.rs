//! Vectors for the `node:crypto` digests, HMAC and random functions. The expected values are the
//! published RFC/NIST ones or were cross-checked against OpenSSL (Python hashlib).

use super::digest::{digest_bytes, hmac_bytes};
use super::kdf::{pbkdf2_bytes, pbkdf2_in_house, scrypt_bytes};
use crate::*;

/// Run `src` and return the console.log lines joined by `|` (or the uncaught error text).
fn run(src: &str) -> String {
    let mut it = Interp::new();
    match it.run(src) {
        Ok(_) => it.out.join("|"),
        Err(Ctl::Throw(v)) => format!("Uncaught {}", it.to_str(&v)),
        Err(_) => "stray completion".into(),
    }
}

fn digest_hex(algorithm: &str, value: &[u8]) -> String {
    crate::zlib::hex_encode(&digest_bytes(algorithm, value).expect("supported digest"))
}

fn hmac_hex(algorithm: &str, key: &[u8], value: &[u8]) -> String {
    crate::zlib::hex_encode(&hmac_bytes(algorithm, key, value).expect("supported digest"))
}

#[test]
fn digests_match_the_published_vectors() {
    assert_eq!(
        digest_hex("sha224", b"abc"),
        "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7"
    );
    assert_eq!(
        digest_hex("sha224", b""),
        "d14a028c2a3a2bc9476102bb288234c415a2b01f828ea62ac5b3e42f"
    );
    assert_eq!(
        digest_hex("sha3224", b"abc"),
        "e642824c3f8cf24ad09234ee7d3c766fc9a3a5168d0c94ad73b46fdf"
    );
    assert_eq!(
        digest_hex("sha3256", b""),
        "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a"
    );
    assert_eq!(
        digest_hex("sha3256", b"abc"),
        "3a985da74fe225b2045c172d6bd390bd855f086e3e9d525b46bfe24511431532"
    );
    assert_eq!(digest_hex("sha3384", b""), "0c63a75b845e4f7d01107d852e4c2485c51a50aaaa94fc61995e71bbee983a2ac3713831264adb47fb6bd1e058d5f004");
    assert_eq!(
        digest_hex("sha3512", b"abc"),
        "b751850b1a57168a5693cd924b6b096e08f621827444f70d884f5d0240d2712e10e116e9192af3c91a7ec57647e3934057340b4cf408d5a56592f8274eec53f0"
    );
    assert_eq!(
        digest_hex("ripemd160", b"abc"),
        "8eb208f7e05d987a9b044a8e98c6b087f15a0bfc"
    );
    assert_eq!(
        digest_hex("ripemd160", b""),
        "9c1185a5c5e9fc54612808977ee8f548b2258d31"
    );
    assert_eq!(
        digest_hex("ripemd160", b"message digest"),
        "5d0689ef49d2fae572b881b123a85ffa21595f36"
    );
    assert_eq!(
        digest_hex("md5", b"abc"),
        "900150983cd24fb0d6963f7d28e17f72"
    );
}

#[test]
fn hmac_matches_the_rfc_vectors_and_long_keys() {
    let jefe = b"what do ya want for nothing?";
    assert_eq!(
        hmac_hex("sha224", b"Jefe", jefe),
        "a30e01098bc6dbbf45690f3a7e9e6d0f8bbea2a39e6148008fd05e44"
    );
    assert_eq!(
        hmac_hex("sha256", b"Jefe", jefe),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
    assert_eq!(
        hmac_hex("sha1", b"Jefe", jefe),
        "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79"
    );
    assert_eq!(
        hmac_hex("md5", &[0x0b; 16], b"Hi There"),
        "9294727a3638bb1c13f48ef8158bfc9d"
    );
    assert_eq!(
        hmac_hex("sha512", b"Jefe", jefe),
        "164b7a7bfcf819e2e395fbe73b56e0a387bd64222e831fd610270cd7ea2505549758bf75c05a994a6d034f65f8f0e6fdcaeab1a34d4a6b4b636e070a38bce737"
    );
    // RFC 4231 test case 6: a 131-byte key is hashed down to the digest size first.
    assert_eq!(
        hmac_hex(
            "sha256",
            &[0xaa; 131],
            b"Test Using Larger Than Block-Size Key - Hash Key First"
        ),
        "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
    );
    assert_eq!(
        hmac_hex("sha3224", b"Jefe", jefe),
        "7fdb8dd88bd2f60d1b798634ad386811c2cfc85bfaf5d52bbace5e66"
    );
    assert_eq!(
        hmac_hex("sha3256", b"Jefe", jefe),
        "c7d4072e788877ae3596bbb0da73b887c9171f93095b294ae857fbe2645e1ba5"
    );
    assert_eq!(hmac_hex("sha3384", b"Jefe", jefe), "f1101f8cbf9766fd6764d2ed61903f21ca9b18f57cf3e1a23ca13508a93243ce48c045dc007f26a21b3f5e0e9df4c20a");
    assert_eq!(hmac_hex("sha3512", b"Jefe", jefe), "5a4bfeab6166427c7a3647b747292b8384537cdb89afb3bf5665e4c5e709350b287baec921fd7ca0ee7a0c31d022a95e1fc92ba9d77df883960275beb4e62024");
    assert_eq!(
        hmac_hex("ripemd160", b"Jefe", jefe),
        "dda6c0213a485a9e24f4742064a7f033b43c4069"
    );
    assert_eq!(
        hmac_hex("ripemd160", &[0xaa; 100], b"x"),
        "fdf6b85da51b973e90d8bff1b5a4febae6ee4920"
    );
}

#[test]
fn unsupported_digest_names_are_rejected() {
    assert!(digest_bytes("sha512224", b"").is_none());
    assert!(digest_bytes("", b"").is_none());
    assert!(hmac_bytes("whirlpool", b"k", b"v").is_none());
}

#[test]
fn hash_and_hmac_objects_match_node_for_aliases_and_views() {
    assert_eq!(
        run(r#"var m = globalThis.__thaw_crypto_module, h = function (a, s) { return m.createHash(a).update(s).digest('hex'); };
             console.log(h('sha224', 'abc'), h('sha3-256', ''), h('SHA3-512', 'abc'), h('rmd160', 'abc'));
             console.log(h('ripemd160', 'message digest'), h('md5', 'abc'), h('sha-1', 'abc'));
             console.log(m.createHmac('sha256', 'Jefe').update('what do ya ').update('want for nothing?').digest('hex'));
             console.log(m.createHmac('sha256', Buffer.alloc(131, 0xaa)).update('Test Using Larger Than Block-Size Key - Hash Key First').digest('hex'));
             var base = new Uint8Array([0, 97, 98, 99, 0]).subarray(1, 4);
             console.log(m.createHash('sha1').update(base).digest('hex'));"#),
        "23097d223405d8228642a477bda255b32aadbce4bda0b3f7e36c9da7 a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a b751850b1a57168a5693cd924b6b096e08f621827444f70d884f5d0240d2712e10e116e9192af3c91a7ec57647e3934057340b4cf408d5a56592f8274eec53f0 8eb208f7e05d987a9b044a8e98c6b087f15a0bfc|5d0689ef49d2fae572b881b123a85ffa21595f36 900150983cd24fb0d6963f7d28e17f72 a9993e364706816aba3e25717850c26c9cd0d89d|5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843|60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54|a9993e364706816aba3e25717850c26c9cd0d89d"
    );
}

#[test]
fn hash_copy_is_independent_and_digest_finalizes() {
    assert_eq!(
        run(r#"var m = globalThis.__thaw_crypto_module;
             var h = m.createHash('sha256').update('a'), c = h.copy();
             c.update('b'); h.update('c');
             console.log(h.digest('hex'));
             console.log(c.digest('hex'));
             var again = ''; try { h.digest(); } catch (e) { again = e.code; }
             var late = ''; try { h.copy(); } catch (e) { late = e.code; }
             var unknown = ''; try { m.createHash('nope'); } catch (e) { unknown = e.message; }
             console.log(again, late);
             console.log(unknown);
             var k = m.createHmac('sha1', 'k').update('x'), kc = k.copy(); kc.update('y');
             console.log(kc.digest('hex') === m.createHmac('sha1', 'k').update('xy').digest('hex'));"#),
        "f45de51cdef30991551e41e882dd7b5404799648a0a00753f44fc966e6153fc1|fb8e20fc2e4c3f248c60c39bd652f3c1347298bb977b8b4d5903b85055620603|ERR_CRYPTO_HASH_FINALIZED ERR_CRYPTO_HASH_FINALIZED|Digest method not supported|true"
    );
}

#[test]
fn crypto_hash_getters_and_big_inputs_match_node() {
    assert_eq!(
        run(r#"var m = globalThis.__thaw_crypto_module;
             console.log(m.hash('sha1', 'abc'), m.hash('sha256', 'abc', 'base64'), m.hash('sha3-256', '', 'buffer').length);
             console.log(m.getHashes().length, m.getHashes().indexOf('sha3-256') >= 0, m.getHashes().indexOf('ripemd160') >= 0, m.getCiphers().length > 50);
             var big = Buffer.alloc(1 << 16, 7);
             var whole = m.createHash('sha3-256').update(big).digest('hex');
             var parts = m.createHash('sha3-256').update(big.subarray(0, 1000)).update(big.subarray(1000)).digest('hex');
             console.log(whole === parts, whole);"#),
        "a9993e364706816aba3e25717850c26c9cd0d89d ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0= 32|52 true true true|true 2bef5111226a770072d110539ac017791f38b16add6d44a0fa0ebe98ce2cbc27"
    );
}

#[test]
fn random_functions_keep_their_shape_bounds_and_async_timing() {
    assert_eq!(
        run(r#"var m = globalThis.__thaw_crypto_module;
             var b = Buffer.alloc(64), r = m.randomFillSync(b, 16, 16);
             var outside = 0; for (var i = 0; i < 64; i++) { if ((i < 16 || i >= 32) && b[i] !== 0) outside++; }
             console.log(r === b, outside, m.randomBytes(5).length, m.randomBytes(0).length);
             var ok = true; for (var j = 0; j < 300; j++) { var v = m.randomInt(5, 10); if (v < 5 || v >= 10 || Math.floor(v) !== v) ok = false; }
             var z = m.randomInt(7), rangeErr = ''; try { m.randomInt(3, 3); } catch (e) { rangeErr = e.name + ':' + e.message; }
             console.log(ok, z >= 0 && z < 7, rangeErr);
             var u = m.randomUUID(), neg = ''; try { m.randomBytes(-1); } catch (e) { neg = e.code; }
             console.log(/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(u), neg);
             m.randomBytes(4, function (e, v) { console.log('cb', e, v.length); });
             console.log('sync');"#),
        "true 0 5 0|true true RangeError:invalid random integer range|true ERR_OUT_OF_RANGE|sync"
    );
}

fn pbkdf2_hex(
    algorithm: &str,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    length: usize,
) -> String {
    crate::zlib::hex_encode(
        &pbkdf2_bytes(algorithm, password, salt, iterations, length).expect("supported digest"),
    )
}

fn scrypt_hex(
    password: &[u8],
    salt: &[u8],
    length: u32,
    cost: u32,
    block_size: u32,
    parallelization: u32,
) -> String {
    crate::zlib::hex_encode(
        &scrypt_bytes(password, salt, length, cost, block_size, parallelization)
            .expect("valid scrypt params"),
    )
}

#[test]
fn pbkdf2_sha1_matches_the_rfc6070_vectors() {
    assert_eq!(
        pbkdf2_hex("sha1", b"password", b"salt", 1, 20),
        "0c60c80f961f0e71f3a9b524af6012062fe037a6"
    );
    assert_eq!(
        pbkdf2_hex("sha1", b"password", b"salt", 2, 20),
        "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957"
    );
    assert_eq!(
        pbkdf2_hex("sha1", b"password", b"salt", 4096, 20),
        "4b007901b765489abead49d926f721d065a429c1"
    );
    // Multi-block output (RFC 6070 vectors 5 and 6).
    assert_eq!(
        pbkdf2_hex(
            "sha1",
            b"passwordPASSWORDpassword",
            b"saltSALTsaltSALTsaltSALTsaltSALTsalt",
            4096,
            25
        ),
        "3d2eec4fe41c849b80c8d83662c0e44a8b291a964cf2f07038"
    );
    assert_eq!(
        pbkdf2_hex("sha1", b"pass\0word", b"sa\0lt", 4096, 16),
        "56fa6aa75548099dcc37d7f03425e0c3"
    );
    assert_eq!(
        pbkdf2_in_house("sha1", b"password", b"salt", 4096, 20)
            .map(|key| crate::zlib::hex_encode(&key))
            .unwrap(),
        "4b007901b765489abead49d926f721d065a429c1"
    );
    assert!(pbkdf2_bytes("nope", b"p", b"s", 1, 8).is_none());
    assert!(pbkdf2_in_house("nope", b"p", b"s", 1, 8).is_none());
}

#[test]
fn pbkdf2_in_house_loop_matches_the_crate_path_and_other_digests() {
    assert_eq!(
        pbkdf2_hex("sha3256", b"password", b"salt", 3, 32),
        "6677065466c97fdef1c15ae8d95020ca948334f53eceeafc6115ddf405f6d6a4"
    );
    assert_eq!(
        pbkdf2_hex("md5", b"password", b"salt", 3, 16),
        "f6acd4bda3e4d3d831a5f61da9ca9d5c"
    );
    assert_eq!(
        pbkdf2_hex("ripemd160", b"pw", b"salt", 3, 40),
        "872082fba2e5e614e84d9290769acc835cdf6bf33bfa77845b62947255cd4aec63aeaeb39b5c379b"
    );
    let in_house =
        pbkdf2_in_house("sha256", b"password", b"salt", 3, 70).expect("supported digest");
    assert_eq!(
        in_house,
        pbkdf2_bytes("sha256", b"password", b"salt", 3, 70).expect("supported digest")
    );
}

#[test]
fn scrypt_matches_the_rfc7914_vectors() {
    assert_eq!(
        scrypt_hex(b"", b"", 64, 16, 1, 1),
        "77d6576238657b203b19ca42c18a0497f16b4844e3074ae8dfdffa3fede21442fcd0069ded0948f8326a753a0fc81f17e8d3e0fb2e0d3628cf35e20c38d18906"
    );
    assert_eq!(
        scrypt_hex(b"password", b"NaCl", 64, 1024, 8, 16),
        "fdbabe1c9d3472007856e7190d01e9fe7c6ad7cbc8237830e77376634b3731622eaf30d92e22a3886ff109279d9830dac727afb94a83ee6d8360cbdfa2cc0640"
    );
}

#[test]
fn scrypt_rejects_bad_cost_and_params_with_the_quickjs_messages() {
    assert_eq!(
        scrypt_bytes(b"", b"", 64, 3, 8, 1).unwrap_err(),
        "Error converting from js 'scrypt cost' into type 'power of two greater than one': 3"
    );
    assert_eq!(
        scrypt_bytes(b"", b"", 64, 1, 8, 1).unwrap_err(),
        "Error converting from js 'scrypt cost' into type 'power of two greater than one': 1"
    );
    assert_eq!(
        scrypt_bytes(b"", b"", 64, 0, 8, 1).unwrap_err(),
        "Error converting from js 'scrypt cost' into type 'power of two greater than one': 0"
    );
    assert_eq!(
        scrypt_bytes(b"", b"", 8, 16, 0, 1).unwrap_err(),
        "Error converting from js 'scrypt options' into type 'valid scrypt options': invalid scrypt parameters: N=16, r=0, p=1, length=8"
    );
    assert_eq!(
        scrypt_bytes(b"", b"", 8, 16, 1, 0).unwrap_err(),
        "Error converting from js 'scrypt options' into type 'valid scrypt options': invalid scrypt parameters: N=16, r=1, p=0, length=8"
    );
}

#[test]
fn pbkdf2_sha256_600k_iterations_stays_within_budget() {
    let started = std::time::Instant::now();
    let key = pbkdf2_bytes("sha256", b"password", b"salt", 600_000, 32).expect("supported digest");
    let elapsed = started.elapsed();
    assert_eq!(
        crate::zlib::hex_encode(&key),
        "669cfe52482116fda1aa2cbe409b2f56c8e4563752b7a28f6eaab614ee005178"
    );
    assert!(
        // ~4 s in a debug build on a quiet host; the guard only has to catch an order-of-magnitude slowdown.
        elapsed.as_secs_f64() < 20.0,
        "pbkdf2-sha256 with 600000 iterations took {elapsed:?}"
    );
}

#[test]
fn kdfs_match_the_vectors_through_the_module() {
    assert_eq!(
        run(r#"var m = globalThis.__thaw_crypto_module;
             console.log(m.pbkdf2Sync('password', 'salt', 2, 20, 'sha1').toString('hex'));
             console.log(m.pbkdf2Sync('password', 'salt', 1, 20, 'SHA-1').toString('hex'));
             console.log(m.scryptSync('', '', 64, { N: 16, r: 1, p: 1 }).toString('hex').slice(0, 32));
             console.log(m.scryptSync('password', 'NaCl', 64, { N: 1024, r: 8, p: 16 }).toString('hex').slice(0, 32));
             console.log(m.hkdfSync('sha256', Buffer.alloc(22, 0x0b), Buffer.from('000102030405060708090a0b0c', 'hex'), Buffer.from('f0f1f2f3f4f5f6f7f8f9', 'hex'), 42).toString('hex'));"#),
        "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957|0c60c80f961f0e71f3a9b524af6012062fe037a6|77d6576238657b203b19ca42c18a0497|fdbabe1c9d3472007856e7190d01e9fe|3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
    );
}

#[test]
fn kdf_argument_errors_match_the_quickjs_messages() {
    assert_eq!(
        run(r#"var m = globalThis.__thaw_crypto_module, t = function (f) { try { f(); return 'ok'; } catch (e) { return e.name + ':' + e.message; } };
             console.log(t(function () { m.pbkdf2Sync('p', 's', 0, 8, 'sha1'); }));
             console.log(t(function () { m.pbkdf2Sync('p', 's', 1, -1, 'sha1'); }));
             console.log(t(function () { m.pbkdf2Sync('p', 's', 4294967296, 8, 'sha1'); }));
             console.log(t(function () { m.pbkdf2Sync('p', 's', 1, 8, 'nope'); }));
             console.log(t(function () { m.scryptSync('p', 's', 8, { N: 3 }); }));
             console.log(t(function () { m.scryptSync('p', 's', 8, { N: 16, r: 0 }); }));
             console.log(t(function () { m.scrypt('p', 's', 8, {}, 1); }));
             console.log(t(function () { m.pbkdf2('p', 's', 1, 8, 'sha1', 1); }));"#),
        "RangeError:iterations out of range|RangeError:keylen out of range|RangeError:iterations out of range|Error:Digest method not supported|TypeError:Error converting from js 'scrypt cost' into type 'power of two greater than one': 3|TypeError:Error converting from js 'scrypt options' into type 'valid scrypt options': invalid scrypt parameters: N=16, r=0, p=1, length=8|TypeError:callback must be a function|TypeError:callback must be a function"
    );
}
