//! Vectors for the `node:crypto` digests, HMAC and random functions. The expected values are the
//! published RFC/NIST ones or were cross-checked against OpenSSL (Python hashlib).

use super::cipher::{sym_block_cipher, sym_ctr_xor, sym_gcm_tag, sym_gcm_xor};
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

fn nist(text: &str) -> Vec<u8> {
    crate::zlib::hex_decode(text)
}

#[test]
fn aes_block_modes_and_ctr_match_nist_sp800_38a_for_every_key_size() {
    // NIST SP 800-38A F.1/F.2/F.5 plaintext (four blocks), keys for AES-128/192/256 (F.x.1/3/5).
    let plain = nist("6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e5130c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710");
    let iv = nist("000102030405060708090a0b0c0d0e0f");
    let ctr_iv = nist("f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff");
    let cases = [
        ("2b7e151628aed2a6abf7158809cf4f3c", "7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b273bed6b8e3c1743b7116e69e222295163ff1caa1681fac09120eca307586e1a7", "3ad77bb40d7a3660a89ecaf32466ef97f5d3d58503b9699de785895a96fdbaaf43b1cd7f598ece23881b00e3ed0306887b0c785e27e8ad3f8223207104725dd4", "874d6191b620e3261bef6864990db6ce9806f66b7970fdff8617187bb9fffdff5ae4df3edbd5d35e5b4f09020db03eab1e031dda2fbe03d1792170a0f3009cee"),
        ("8e73b0f7da0e6452c810f32b809079e562f8ead2522c6b7b", "4f021db243bc633d7178183a9fa071e8b4d9ada9ad7dedf4e5e738763f69145a571b242012fb7ae07fa9baac3df102e008b0e27988598881d920a9e64f5615cd", "bd334f1d6e45f25ff712a214571fa5cc974104846d0ad3ad7734ecb3ecee4eefef7afd2270e2e60adce0ba2face6444e9a4b41ba738d6c72fb16691603c18e0e", "1abc932417521ca24f2b0459fe7e6e0b090339ec0aa6faefd5ccc2c6f4ce8e941e36b26bd1ebc670d1bd1d665620abf74f78a7f6d29809585a97daec58c6b050"),
        ("603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4", "f58c4c04d6e5f1ba779eabfb5f7bfbd69cfc4e967edb808d679f777bc6702c7d39f23369a9d9bacfa530e26304231461b2eb05e2c39be9fcda6c19078c6a9d1b", "f3eed1bdb5d2a03c064b5a7e3db181f8591ccb10d410ed26dc5ba74a31362870b6ed21b99ca6f4f9f153e7b1beafed1d23304b7a39f9f3ff067d8d8f9e24ecc7", "601ec313775789a5b7a7f504bbf3d228f443e3ca4d62b59aca84e990cacaf5c52b0930daa23de94ce87017ba2d84988ddfc9c58db67aada613c2dd08457941a6"),
    ];
    for (key, cbc, ecb, ctr) in cases {
        let (key, cbc, ecb, ctr) = (nist(key), nist(cbc), nist(ecb), nist(ctr));
        assert_eq!(
            sym_block_cipher(false, &key, &iv, &plain, false, false, false).unwrap(),
            cbc
        );
        assert_eq!(
            sym_block_cipher(false, &key, &iv, &cbc, true, false, false).unwrap(),
            plain
        );
        assert_eq!(
            sym_block_cipher(true, &key, &[], &plain, false, false, false).unwrap(),
            ecb
        );
        assert_eq!(
            sym_block_cipher(true, &key, &[], &ecb, true, false, false).unwrap(),
            plain
        );
        assert_eq!(sym_ctr_xor(&key, &ctr_iv, 0, &plain, true).unwrap(), ctr);
    }
    assert_eq!(
        sym_block_cipher(true, &[0u8; 15], &[], &[0u8; 16], false, false, false),
        Err("Invalid key length".to_string())
    );
}

#[test]
fn gcm_test_case_two_matches_nist() {
    let (key, iv, plain) = (vec![0u8; 16], vec![0u8; 12], vec![0u8; 16]);
    let cipher = sym_gcm_xor(&key, &iv, 0, &plain).unwrap();
    assert_eq!(
        crate::zlib::hex_encode(&cipher),
        "0388dace60b6a392f328c2b971b2fe78"
    );
    assert_eq!(
        crate::zlib::hex_encode(&sym_gcm_tag(&key, &iv, &[], &cipher).unwrap()),
        "ab6e47d42cec13bdf53a67b21257bddf"
    );
}

#[test]
fn cipher_known_answers_and_round_trips_match_node() {
    assert_eq!(
        run(r#"
var m = globalThis.__thaw_crypto_module;
var pt = Buffer.from('6bc1bee22e409f96e93d7e117393172aae2d8a571e03ac9c9eb76fac45af8e5130c81c46a35ce411e5fbc1191a0a52eff69f2445df4f9b17ad2b417be66c3710', 'hex');
var iv = Buffer.from('000102030405060708090a0b0c0d0e0f', 'hex');
var ctrIv = Buffer.from('f0f1f2f3f4f5f6f7f8f9fafbfcfdfeff', 'hex');
var keys = { 128: '2b7e151628aed2a6abf7158809cf4f3c', 192: '8e73b0f7da0e6452c810f32b809079e562f8ead2522c6b7b', 256: '603deb1015ca71be2b73aef0857d77811f352c073b6108d72d9810a30914dff4' };
[128, 192, 256].forEach(function (bits) {
  var key = Buffer.from(keys[bits], 'hex');
  var c = m.createCipheriv('aes-' + bits + '-cbc', key, iv); c.setAutoPadding(false);
  var ct = Buffer.concat([c.update(pt), c.final()]);
  console.log(ct.toString('hex'));
  var d = m.createDecipheriv('aes-' + bits + '-cbc', key, iv); d.setAutoPadding(false);
  console.log(Buffer.concat([d.update(ct), d.final()]).equals(pt));
  var e = m.createCipheriv('aes-' + bits + '-ecb', key, null); e.setAutoPadding(false);
  console.log(Buffer.concat([e.update(pt), e.final()]).toString('hex'));
  var t = m.createCipheriv('aes-' + bits + '-ctr', key, ctrIv);
  console.log(Buffer.concat([t.update(pt), t.final()]).toString('hex'));
});
"#),
        "7649abac8119b246cee98e9b12e9197d5086cb9b507219ee95db113a917678b273bed6b8e3c1743b7116e69e222295163ff1caa1681fac09120eca307586e1a7|true|3ad77bb40d7a3660a89ecaf32466ef97f5d3d58503b9699de785895a96fdbaaf43b1cd7f598ece23881b00e3ed0306887b0c785e27e8ad3f8223207104725dd4|874d6191b620e3261bef6864990db6ce9806f66b7970fdff8617187bb9fffdff5ae4df3edbd5d35e5b4f09020db03eab1e031dda2fbe03d1792170a0f3009cee|4f021db243bc633d7178183a9fa071e8b4d9ada9ad7dedf4e5e738763f69145a571b242012fb7ae07fa9baac3df102e008b0e27988598881d920a9e64f5615cd|true|bd334f1d6e45f25ff712a214571fa5cc974104846d0ad3ad7734ecb3ecee4eefef7afd2270e2e60adce0ba2face6444e9a4b41ba738d6c72fb16691603c18e0e|1abc932417521ca24f2b0459fe7e6e0b090339ec0aa6faefd5ccc2c6f4ce8e941e36b26bd1ebc670d1bd1d665620abf74f78a7f6d29809585a97daec58c6b050|f58c4c04d6e5f1ba779eabfb5f7bfbd69cfc4e967edb808d679f777bc6702c7d39f23369a9d9bacfa530e26304231461b2eb05e2c39be9fcda6c19078c6a9d1b|true|f3eed1bdb5d2a03c064b5a7e3db181f8591ccb10d410ed26dc5ba74a31362870b6ed21b99ca6f4f9f153e7b1beafed1d23304b7a39f9f3ff067d8d8f9e24ecc7|601ec313775789a5b7a7f504bbf3d228f443e3ca4d62b59aca84e990cacaf5c52b0930daa23de94ce87017ba2d84988ddfc9c58db67aada613c2dd08457941a6"
    );
}

#[test]
fn gcm_case_two_and_aad_round_trip_match_node() {
    assert_eq!(
        run(r#"
var m = globalThis.__thaw_crypto_module;
var zero16 = Buffer.alloc(16), iv12 = Buffer.alloc(12);
var c = m.createCipheriv('aes-128-gcm', zero16, iv12);
var ct = Buffer.concat([c.update(zero16), c.final()]);
console.log(ct.toString('hex'));
console.log(c.getAuthTag().toString('hex'));
var d = m.createDecipheriv('aes-128-gcm', zero16, iv12);
d.setAuthTag(c.getAuthTag());
console.log(Buffer.concat([d.update(ct), d.final()]).toString('hex'));
var g = m.createCipheriv('aes-192-gcm', Buffer.alloc(24, 3), Buffer.alloc(12, 4));
g.setAAD(Buffer.from('header'));
var aadCt = Buffer.concat([g.update('secret message here'), g.final()]);
console.log(aadCt.toString('hex'));
console.log(g.getAuthTag().toString('hex'));
var h = m.createDecipheriv('aes-192-gcm', Buffer.alloc(24, 3), Buffer.alloc(12, 4));
h.setAAD(Buffer.from('header'));
h.setAuthTag(g.getAuthTag());
console.log(Buffer.concat([h.update(aadCt), h.final()]).toString());
var odd = m.createCipheriv('aes-128-gcm', Buffer.alloc(16, 5), Buffer.alloc(9, 6));
var msg = Buffer.from('The quick brown fox jumps over the lazy dog, twice over!!');
console.log(Buffer.concat([odd.update(msg), odd.final()]).toString('hex'));
console.log(odd.getAuthTag().toString('hex'));
"#),
        "0388dace60b6a392f328c2b971b2fe78|ab6e47d42cec13bdf53a67b21257bddf|00000000000000000000000000000000|65fef97960e44934314d5d6009e1bb42430939|ffbd1293459d03e83519001dcda3382d|secret message here|fa74eeb208bc982e5c219f2036473eaea336b89ea498929830affd2b722e8021b4a98e35cc7bf0598130f4a136b18e4d3af4c69f267f9b3dec|34c146485ed83b7f5154c95366e59987"
    );
}

#[test]
fn cipher_argument_and_final_errors_match_node_codes() {
    assert_eq!(
        run(r#"
var m = globalThis.__thaw_crypto_module;
var t = function (f) { try { var r = f(); return 'ok:' + (Buffer.isBuffer(r) ? r.toString('hex') : r); } catch (e) { return e.name + '|' + e.code + '|' + e.message; } };
var k16 = Buffer.alloc(16), iv12 = Buffer.alloc(12);
console.log(t(function () { return m.createCipheriv('aes-128-cbc', Buffer.alloc(15), Buffer.alloc(16)); }));
console.log(t(function () { return m.createCipheriv('aes-256-cbc', Buffer.alloc(16), Buffer.alloc(16)); }));
console.log(t(function () { return m.createCipheriv('aes-128-cbc', k16, Buffer.alloc(8)); }));
console.log(t(function () { return m.createCipheriv('aes-128-ecb', k16, Buffer.alloc(16)); }));
console.log(t(function () { return m.createCipheriv('aes-128-gcm', k16, Buffer.alloc(0)); }));
console.log(t(function () { return m.createCipheriv('aes-128-foo', k16, Buffer.alloc(16)); }));
console.log(t(function () { return m.createCipheriv('aes-128-cbc', k16, Buffer.alloc(16)).getAuthTag(); }));
console.log(t(function () { return m.createCipheriv('aes-128-gcm', k16, iv12).getAuthTag(); }));
var nopad = m.createCipheriv('aes-128-cbc', k16, Buffer.alloc(16)); nopad.setAutoPadding(false); nopad.update(Buffer.alloc(10));
console.log(t(function () { return nopad.final(); }));
var enc = m.createCipheriv('aes-128-cbc', k16, Buffer.alloc(16));
var e1 = Buffer.concat([enc.update('hello world'), enc.final()]);
var wrong = m.createDecipheriv('aes-128-cbc', Buffer.alloc(16, 2), Buffer.alloc(16)); wrong.update(e1);
console.log(t(function () { return wrong.final(); }));
var badPad = m.createDecipheriv('aes-128-cbc', k16, Buffer.alloc(16)); badPad.update(Buffer.alloc(16));
console.log(t(function () { return badPad.final(); }));
var gcmBad = m.createDecipheriv('aes-128-gcm', k16, iv12); gcmBad.setAuthTag(Buffer.alloc(16, 1)); gcmBad.update(Buffer.alloc(16));
console.log(t(function () { return gcmBad.final(); }));
var gcmNone = m.createDecipheriv('aes-128-gcm', k16, iv12); gcmNone.update(Buffer.alloc(16));
console.log(t(function () { return gcmNone.final(); }));
"#),
        "RangeError|ERR_CRYPTO_INVALID_KEYLEN|Invalid key length|RangeError|ERR_CRYPTO_INVALID_KEYLEN|Invalid key length|TypeError|ERR_CRYPTO_INVALID_IV|Invalid initialization vector|TypeError|ERR_CRYPTO_INVALID_IV|Invalid initialization vector|TypeError|ERR_CRYPTO_INVALID_IV|Invalid initialization vector|Error|ERR_CRYPTO_UNKNOWN_CIPHER|Unknown cipher|Error|ERR_CRYPTO_INVALID_STATE|Invalid state for operation getAuthTag|Error|ERR_CRYPTO_INVALID_STATE|Invalid state for operation getAuthTag|Error|ERR_OSSL_WRONG_FINAL_BLOCK_LENGTH|error:1C80006B:Provider routines::wrong final block length|Error|ERR_OSSL_BAD_DECRYPT|error:1C800064:Provider routines::bad decrypt|Error|ERR_OSSL_BAD_DECRYPT|error:1C800064:Provider routines::bad decrypt|Error|undefined|Unsupported state or unable to authenticate data|Error|undefined|Unsupported state or unable to authenticate data"
    );
}

#[test]
fn update_then_final_chunking_and_output_sizes_match_node() {
    assert_eq!(
        run(r#"
var m = globalThis.__thaw_crypto_module;
var msg = Buffer.from('The quick brown fox jumps over the lazy dog, twice over!!');
function parts(c, size) { var out = []; for (var i = 0; i < msg.length; i += size) out.push(c.update(msg.subarray(i, i + size))); out.push(c.final()); return Buffer.concat(out).toString('hex'); }
var one = function (alg, key, iv) { var c = m.createCipheriv(alg, key, iv); return Buffer.concat([c.update(msg), c.final()]).toString('hex'); };
var key32 = Buffer.alloc(32, 9), iv16 = Buffer.alloc(16, 8);
console.log(parts(m.createCipheriv('aes-256-cbc', key32, iv16), 7) === one('aes-256-cbc', key32, iv16));
console.log(one('aes-256-cbc', key32, iv16));
console.log(parts(m.createCipheriv('aes-256-ctr', key32, iv16), 5) === one('aes-256-ctr', key32, iv16));
console.log(one('aes-256-ctr', key32, iv16));
var c = m.createCipheriv('aes-128-cbc', Buffer.alloc(16), Buffer.alloc(16));
console.log(c.update(Buffer.alloc(20)).length + '|' + c.update(Buffer.alloc(12)).length + '|' + c.final().length);
var enc = m.createCipheriv('aes-128-cbc', Buffer.alloc(16), Buffer.alloc(16));
var ctx = Buffer.concat([enc.update(Buffer.alloc(20)), enc.final()]);
var dec = m.createDecipheriv('aes-128-cbc', Buffer.alloc(16), Buffer.alloc(16));
console.log(dec.update(ctx).length + '|' + dec.final().length);
console.log(m.createCipheriv('aes-128-ecb', Buffer.alloc(16), null).update(Buffer.alloc(16)).toString('hex'));
var odd = m.createCipheriv('aes-128-gcm', Buffer.alloc(16, 5), Buffer.alloc(9, 6));
console.log(one('aes-128-gcm', Buffer.alloc(16, 5), Buffer.alloc(9, 6)));
"#),
        "true|a228d17c61bddab3b630578c8deac2c796828239b4e0d1e1c3b1f293271efc9f17a209752304547052cb82b29af6aede626ad86912270fdabacfc7502467f112|true|f49e76834fae9a6ef556300d5fbaf43bc587bcc2f76f64ee2f6f54996470ed10e692d3d62399a6f4ba8b08fc5c18bb5c96c738a870db4159f8|16|16|16|16|4|66e94bd4ef8a2c3b884cfa59ca342b2e|fa74eeb208bc982e5c219f2036473eaea336b89ea498929830affd2b722e8021b4a98e35cc7bf0598130f4a136b18e4d3af4c69f267f9b3dec"
    );
}

#[test]
fn timing_safe_equal_matches_node_for_views_and_errors() {
    assert_eq!(
        run(r#"
var m = globalThis.__thaw_crypto_module;
var t = function (f) { try { return String(f()); } catch (e) { return e.name + '|' + e.code + '|' + e.message; } };
console.log(t(function () { return m.timingSafeEqual(Buffer.from('ab'), Buffer.from('ab')); }));
console.log(t(function () { return m.timingSafeEqual(Buffer.from('ab'), Buffer.from('ac')); }));
console.log(t(function () { return m.timingSafeEqual(Buffer.from('ab'), Buffer.from('a')); }));
console.log(t(function () { return m.timingSafeEqual('abc', Buffer.from('abc')); }));
console.log(t(function () { return m.timingSafeEqual(Buffer.from('a'), 'a'); }));
console.log(t(function () { return m.timingSafeEqual(1, Buffer.from('a')); }));
console.log(t(function () { return m.timingSafeEqual(null, Buffer.from('a')); }));
console.log(t(function () { return m.timingSafeEqual(undefined, Buffer.from('a')); }));
console.log(t(function () { return m.timingSafeEqual({}, Buffer.from('a')); }));
console.log(t(function () { return m.timingSafeEqual(Buffer.from('a'), [1]); }));
console.log(t(function () { return m.timingSafeEqual(new ArrayBuffer(2), new Uint8Array([0, 0])); }));
console.log(t(function () { return m.timingSafeEqual(new DataView(new ArrayBuffer(2)), new Uint16Array(1)); }));
console.log(t(function () { return m.timingSafeEqual(new Uint16Array([1]), new Uint8Array([1, 0])); }));
console.log(t(function () { return m.timingSafeEqual(Buffer.from('a'), Buffer.from('b'), 1); }));
console.log(t(function () { return m.timingSafeEqual(Buffer.from('a'), new Uint8Array([97])); }));
console.log(t(function () { return m.timingSafeEqual(Buffer.from('a'), new ArrayBuffer(0)); }));
console.log(t(function () { return m.timingSafeEqual(new Uint8Array(0), Buffer.alloc(0)); }));
console.log(t(function () { return m.timingSafeEqual(Buffer.from('abc'), Buffer.from('abcd').subarray(0, 3)); }));
console.log(t(function () { return m.timingSafeEqual(Buffer.from('a'), 5); }));
"#),
        "true|false|RangeError|ERR_CRYPTO_TIMING_SAFE_EQUAL_LENGTH|Input buffers must have the same byte length|TypeError|ERR_INVALID_ARG_TYPE|The \"buf1\" argument must be an instance of ArrayBuffer, Buffer, TypedArray, or DataView.|TypeError|ERR_INVALID_ARG_TYPE|The \"buf2\" argument must be an instance of ArrayBuffer, Buffer, TypedArray, or DataView.|TypeError|ERR_INVALID_ARG_TYPE|The \"buf1\" argument must be an instance of ArrayBuffer, Buffer, TypedArray, or DataView.|TypeError|ERR_INVALID_ARG_TYPE|The \"buf1\" argument must be an instance of ArrayBuffer, Buffer, TypedArray, or DataView.|TypeError|ERR_INVALID_ARG_TYPE|The \"buf1\" argument must be an instance of ArrayBuffer, Buffer, TypedArray, or DataView.|TypeError|ERR_INVALID_ARG_TYPE|The \"buf1\" argument must be an instance of ArrayBuffer, Buffer, TypedArray, or DataView.|TypeError|ERR_INVALID_ARG_TYPE|The \"buf2\" argument must be an instance of ArrayBuffer, Buffer, TypedArray, or DataView.|true|true|true|false|true|RangeError|ERR_CRYPTO_TIMING_SAFE_EQUAL_LENGTH|Input buffers must have the same byte length|true|true|TypeError|ERR_INVALID_ARG_TYPE|The \"buf2\" argument must be an instance of ArrayBuffer, Buffer, TypedArray, or DataView."
    );
}

#[test]
fn asym_key_import_formats_match_node() {
    let program = [
        r##"function rsaPrivatePkcs8() { return '-----BEGIN PRIVATE KEY-----\nMIIEugIBADANBgkqhkiG9w0BAQEFAASCBKQwggSgAgEAAoIBAQCwwf4NV+7sbU0z\nwaL5V1ABx9epdw571f4wNuNcic8x7CRchIv4DuDlKrHsI6T0y8+pKMdq4rHrDeEt\neeTylrgK7HnqKDe31b8Cxr7BkDLlandEmP3/VPQu7EFDN70rF68IdMn/y9ywbpwY\nNENCIyjyRkfraM5Lr5sBVKjfma8Kfu1PdoE3iMmJX2zhQXQjlnCVAEvelZfSRh64\nTmBv8XoB3dp93HQmRlvpuEObVi6p0wViMYueSaYFviqJONtnwu9xRVBiXXg9qKz6\nsDsRwBuNQTgmCpAXsgwcKmIjl1pvEAroln+LTB3HL20aIJvTD1doJ4Wq3N/17Z5p\ni4jouRUNAgMBAAECgf9ClNB1KqSVMFEkbdIeyNZgMmzKQE6XS36dE4Q/6NfXtkSf\nCWX14gH38+jXSn6vzrj71qCNYp0LFmO2FOjGBCL9+mdPJCiTBYJcjy9XaNzaazp2\nXIgLFJ3mbAxvKGcfQumxqKmdCJCqWV0KY+s3vobLGUV+EDDrF286i0ZulqnHZtAO\nkGGTXsptC86gZHaRYsIuZ0US5KYC1h1tGmwbISdvmfwaXm7N5XW2qU5W3VUZYx4Q\nwjiZ+JJ5RddKYM8TIvK6+AGPqcLdqJxDp/rr5jb0MVJsv+m9BkWRVDSBvnp+zE+b\nJWvUMRc3FeMwqxYAzWVpfsUFUDuc/94fuL6Z68kCgYEA6zuo1Jbe8yEFd8KwJvEj\nN6S7OYmaud9wtKfLTJLa0OmkiXqd88Lw1Mf8uvb4TBP+IOS0XwJYL/R2dmBix4TY\neMesaK1h9twsbKp86sq2bDL5SbT/cWLI6xpgV27vnYIcXKHdtO48FbUrYriTf1sb\n8XYGKAxygcjYewYiLAow6MkCgYEAwFzGDj2HntZaLvHAi/10d7CMhF1bLKNK2l/Q\n46TW+xjAjlAToQ1T+2gfhEvTQ0b+nLJt/ld/FqTogT/3TKpoFG3YWlLZ8ruoPOXY\nxBhL/kxZLchmHN45hzXkd91lOlRLf9TsfHxsYEkOU56kkpIw9zSbHxHbTgUgpwM+\nB5qT8CUCgYB9uuyZfG58O1kl0uy+U8MEGctsjI0j7jbaiJkUO6YzZb5pMR29zaNV\nx/Lgp+K9Hy6EvFlgMuuZ7itnSEtj4zClFeykIpArFzGzf0i3YlQw7unpqJGkNC25\n4+Y8tXHjmUi5hlbvPyrkW2puIMPNnZAI9pGB1G1by1NSJkwbh/LuaQKBgCBB7nyI\n2OtL6seghrdzA0rm8kloFlf/8hd4peDmzZ5B4lh7GS+SupiYN2DKDl1j1GKWkVdr\neMZlVRAHmALlOJrkaLmM1zubOHUt3hHUOTolt3az+luw8Fi6Mtve5pDHffmrzRR7\nEPl8hsiC+/oQReHOkoy9Q9driLQ5GPfRdil5AoGAX64TOs4kpbzM045OgM65Cm2t\nW+SreofkSv2ErT4dzAoPhs0x1xX6BUDNCqn74einRfr1NBJj/llkbMiD8QAHJNq2\npqS4IynQWgf4hYsJlrvG0F79WWadArLVQNLxebGxThzx6l+mIm1wGn/xHibbYBB2\nJz01Ou8GC/69JgjMjas=\n-----END PRIVATE KEY-----\n'; }
function rsaPrivatePkcs1() { return '-----BEGIN RSA PRIVATE KEY-----\nMIIEoAIBAAKCAQEAsMH+DVfu7G1NM8Gi+VdQAcfXqXcOe9X+MDbjXInPMewkXISL\n+A7g5Sqx7COk9MvPqSjHauKx6w3hLXnk8pa4Cux56ig3t9W/Asa+wZAy5Wp3RJj9\n/1T0LuxBQze9KxevCHTJ/8vcsG6cGDRDQiMo8kZH62jOS6+bAVSo35mvCn7tT3aB\nN4jJiV9s4UF0I5ZwlQBL3pWX0kYeuE5gb/F6Ad3afdx0JkZb6bhDm1YuqdMFYjGL\nnkmmBb4qiTjbZ8LvcUVQYl14Pais+rA7EcAbjUE4JgqQF7IMHCpiI5dabxAK6JZ/\ni0wdxy9tGiCb0w9XaCeFqtzf9e2eaYuI6LkVDQIDAQABAoH/QpTQdSqklTBRJG3S\nHsjWYDJsykBOl0t+nROEP+jX17ZEnwll9eIB9/Po10p+r864+9agjWKdCxZjthTo\nxgQi/fpnTyQokwWCXI8vV2jc2ms6dlyICxSd5mwMbyhnH0LpsaipnQiQqlldCmPr\nN76GyxlFfhAw6xdvOotGbpapx2bQDpBhk17KbQvOoGR2kWLCLmdFEuSmAtYdbRps\nGyEnb5n8Gl5uzeV1tqlOVt1VGWMeEMI4mfiSeUXXSmDPEyLyuvgBj6nC3aicQ6f6\n6+Y29DFSbL/pvQZFkVQ0gb56fsxPmyVr1DEXNxXjMKsWAM1laX7FBVA7nP/eH7i+\nmevJAoGBAOs7qNSW3vMhBXfCsCbxIzekuzmJmrnfcLSny0yS2tDppIl6nfPC8NTH\n/Lr2+EwT/iDktF8CWC/0dnZgYseE2HjHrGitYfbcLGyqfOrKtmwy+Um0/3FiyOsa\nYFdu752CHFyh3bTuPBW1K2K4k39bG/F2BigMcoHI2HsGIiwKMOjJAoGBAMBcxg49\nh57WWi7xwIv9dHewjIRdWyyjStpf0OOk1vsYwI5QE6ENU/toH4RL00NG/pyybf5X\nfxak6IE/90yqaBRt2FpS2fK7qDzl2MQYS/5MWS3IZhzeOYc15HfdZTpUS3/U7Hx8\nbGBJDlOepJKSMPc0mx8R204FIKcDPgeak/AlAoGAfbrsmXxufDtZJdLsvlPDBBnL\nbIyNI+422oiZFDumM2W+aTEdvc2jVcfy4KfivR8uhLxZYDLrme4rZ0hLY+MwpRXs\npCKQKxcxs39It2JUMO7p6aiRpDQtuePmPLVx45lIuYZW7z8q5FtqbiDDzZ2QCPaR\ngdRtW8tTUiZMG4fy7mkCgYAgQe58iNjrS+rHoIa3cwNK5vJJaBZX//IXeKXg5s2e\nQeJYexkvkrqYmDdgyg5dY9RilpFXa3jGZVUQB5gC5Tia5Gi5jNc7mzh1Ld4R1Dk6\nJbd2s/pbsPBYujLb3uaQx335q80UexD5fIbIgvv6EEXhzpKMvUPXa4i0ORj30XYp\neQKBgF+uEzrOJKW8zNOOToDOuQptrVvkq3qH5Er9hK0+HcwKD4bNMdcV+gVAzQqp\n++Hop0X69TQSY/5ZZGzIg/EAByTatqakuCMp0FoH+IWLCZa7xtBe/VlmnQKy1UDS\n8XmxsU4c8epfpiJtcBp/8R4m22AQdic9NTrvBgv+vSYIzI2r\n-----END RSA PRIVATE KEY-----\n'; }
function rsaPublicKey() { return '-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsMH+DVfu7G1NM8Gi+VdQ\nAcfXqXcOe9X+MDbjXInPMewkXISL+A7g5Sqx7COk9MvPqSjHauKx6w3hLXnk8pa4\nCux56ig3t9W/Asa+wZAy5Wp3RJj9/1T0LuxBQze9KxevCHTJ/8vcsG6cGDRDQiMo\n8kZH62jOS6+bAVSo35mvCn7tT3aBN4jJiV9s4UF0I5ZwlQBL3pWX0kYeuE5gb/F6\nAd3afdx0JkZb6bhDm1YuqdMFYjGLnkmmBb4qiTjbZ8LvcUVQYl14Pais+rA7EcAb\njUE4JgqQF7IMHCpiI5dabxAK6JZ/i0wdxy9tGiCb0w9XaCeFqtzf9e2eaYuI6LkV\nDQIDAQAB\n-----END PUBLIC KEY-----\n'; }
function ecPrivatePkcs8() { return '-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgHqV2FCeyxUa9ivDr\nlBxTfnDLAsoI5yqk63+Gu/wHFFehRANCAARuk+iU6lIfCrZ55skHVPDCLJokBINo\nHetn8HlbmnNJwQvHw0cTM3BelUyYVXZkLPW5kCLhhNiRpRgGOm+3vLOI\n-----END PRIVATE KEY-----\n'; }
function ecPrivateSec1() { return '-----BEGIN EC PRIVATE KEY-----\nMHcCAQEEIB6ldhQnssVGvYrw65QcU35wywLKCOcqpOt/hrv8BxRXoAoGCCqGSM49\nAwEHoUQDQgAEbpPolOpSHwq2eebJB1TwwiyaJASDaB3rZ/B5W5pzScELx8NHEzNw\nXpVMmFV2ZCz1uZAi4YTYkaUYBjpvt7yziA==\n-----END EC PRIVATE KEY-----\n'; }
function ecPublicKey() { return '-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEbpPolOpSHwq2eebJB1TwwiyaJASD\naB3rZ/B5W5pzScELx8NHEzNwXpVMmFV2ZCz1uZAi4YTYkaUYBjpvt7yziA==\n-----END PUBLIC KEY-----\n'; }
function rsaRoundTrip() {
const sig = __thaw_crypto_module.createSign('RSA-SHA256').update('hello world').sign(rsaPrivatePkcs8(), 'hex');
const ok = __thaw_crypto_module.createVerify('RSA-SHA256').update('hello world').verify(rsaPublicKey(), sig, 'hex');
const okPkcs1Key = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(rsaPublicKey(), __thaw_crypto_module.createSign('sha256').update('hello world').sign(rsaPrivatePkcs1(), 'hex'), 'hex');
const tamperedMessage = __thaw_crypto_module.createVerify('sha256').update('goodbye world').verify(rsaPublicKey(), sig, 'hex');
const tamperedSignature = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(rsaPublicKey(), sig.slice(0, -2) + '00', 'hex');
const opensslSig = 'a7e86b1a31fe823a693f5fedd193ebfb60098de3e41bac389e9e52215d785ee5cc3d211560be5dd87cbadbc264bdea375f96e87420f71b30891352d9a7631e2e1d42d0f8422a4f9af640b3a41617d754e7bdc1738436ac81fa79f6455ed431ab9221671d9582687bf3999ecf618085426bcc3a41c69ec89d0aedcbfaa835d37a89e215859d85d4509afbfb22b569318e779193764f63b9dabfb11c5360b08c2bfcfe257ab5f08f1438f397b2bf284f46848c8ff660ead19e35c67727fef275baf9d91fe56f7a9544f04dcc1ed1152bf737c17a141aaca770fdf4241cef76141722d8910a178b8ca3f183502d49a0c8cf51443e2a797b4ee6ad2be49aea9c9e40';
const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(rsaPublicKey(), opensslSig, 'hex');
return [ok, okPkcs1Key, tamperedMessage, tamperedSignature, opensslVerifies];
}
function rsaPssRoundTrip() {
const sig = __thaw_crypto_module.createSign('sha256').update('hello world').sign({ key: rsaPrivatePkcs8(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PSS_PADDING }, 'hex');
const ok = __thaw_crypto_module.createVerify('sha256').update('hello world').verify({ key: rsaPublicKey(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PSS_PADDING }, sig, 'hex');
const opensslSig = '37ceb5072ae254254115b5910f7a8e45723f1a6ca203eb2084074ab4f95672a0c9d6db585f5bed06cef0208bc5ed474ae0177422928739b05eb40ea4dc2da6204bb73d3e88174f32e01cb94b21b317615e61b8df7e561303261f0868ff704013b20f976c8a77ab5a802f15348b0969a35e57802cd7068f7a386481831c43f19bb511882e5163368ef25f93f73bf47f4374483772dddbb8ca2c03defdb5c4fca89549fea202fab7bf5ca82fc55953119781219ff67b58689866466f2c2bddb4b2f152244b8562fd5e13e3e5e087137e5835cd53978b10f4d4dd37e57ee4aff83dafc1973b0600a780938bddc559bd57b46cdc099895fbd284963a67fe0e9d120b';
const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify({ key: rsaPublicKey(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PSS_PADDING }, opensslSig, 'hex');
return [ok, opensslVerifies];
}
function ecdsaRoundTrip() {
const sig = __thaw_crypto_module.createSign('sha256').update('hello world').sign(ecPrivatePkcs8(), 'hex');
const ok = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(ecPublicKey(), sig, 'hex');
const okSec1Key = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(ecPublicKey(), __thaw_crypto_module.createSign('sha256').update('hello world').sign(ecPrivateSec1(), 'hex'), 'hex');
const tamperedMessage = __thaw_crypto_module.createVerify('sha256').update('goodbye world').verify(ecPublicKey(), sig, 'hex');
const opensslSig = '304502210091b5f1192a836c1d9582197cab6ceac74a952a794d1276781e8f1ac1cf9a001b02203c65bd65ec0b91e24d1f5b52ccaa466e28748e2e56b944d5e7a9aec1bc712210';
const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(ecPublicKey(), opensslSig, 'hex');
return [ok, okSec1Key, tamperedMessage, opensslVerifies];
}
function oneShotSignVerify() {
const sig = __thaw_crypto_module.sign('sha256', 'one shot', rsaPrivatePkcs8());
return __thaw_crypto_module.verify('sha256', 'one shot', rsaPublicKey(), sig);
}
function keyObjectAsymmetricInfo() {
const priv = __thaw_crypto_module.createPrivateKey(rsaPrivatePkcs8());
const pub = __thaw_crypto_module.createPublicKey(rsaPublicKey());
const ecPub = __thaw_crypto_module.createPublicKey(ecPublicKey());
return [priv.type, priv.asymmetricKeyType, pub.type, pub.asymmetricKeyType, ecPub.asymmetricKeyType, ecPub.asymmetricKeyDetails.namedCurve];
}"##,
        r##"
var m = __thaw_crypto_module;
var rp = m.createPrivateKey(rsaPrivatePkcs1());
var rk = m.createPrivateKey(rsaPrivatePkcs8());
console.log(JSON.stringify([rp.type, rp.asymmetricKeyType, rk.asymmetricKeyType,
  String(rk.asymmetricKeyDetails.modulusLength), String(rk.asymmetricKeyDetails.publicExponent),
  m.createPublicKey(rsaPublicKey()).asymmetricKeyType, m.createPublicKey(ecPublicKey()).asymmetricKeyDetails.namedCurve,
  m.createPrivateKey(ecPrivateSec1()).asymmetricKeyType, rp.export().indexOf('-----BEGIN PRIVATE KEY-----') === 0]));
"##,
    ]
    .concat();
    assert_eq!(
        run(&program),
        r#"["private","rsa","rsa","2048","65537","rsa","prime256v1","ec",true]"#
    );
}

#[test]
fn asym_ecdsa_p256_sign_verify_matches_openssl() {
    let program = [
        r##"function rsaPrivatePkcs8() { return '-----BEGIN PRIVATE KEY-----\nMIIEugIBADANBgkqhkiG9w0BAQEFAASCBKQwggSgAgEAAoIBAQCwwf4NV+7sbU0z\nwaL5V1ABx9epdw571f4wNuNcic8x7CRchIv4DuDlKrHsI6T0y8+pKMdq4rHrDeEt\neeTylrgK7HnqKDe31b8Cxr7BkDLlandEmP3/VPQu7EFDN70rF68IdMn/y9ywbpwY\nNENCIyjyRkfraM5Lr5sBVKjfma8Kfu1PdoE3iMmJX2zhQXQjlnCVAEvelZfSRh64\nTmBv8XoB3dp93HQmRlvpuEObVi6p0wViMYueSaYFviqJONtnwu9xRVBiXXg9qKz6\nsDsRwBuNQTgmCpAXsgwcKmIjl1pvEAroln+LTB3HL20aIJvTD1doJ4Wq3N/17Z5p\ni4jouRUNAgMBAAECgf9ClNB1KqSVMFEkbdIeyNZgMmzKQE6XS36dE4Q/6NfXtkSf\nCWX14gH38+jXSn6vzrj71qCNYp0LFmO2FOjGBCL9+mdPJCiTBYJcjy9XaNzaazp2\nXIgLFJ3mbAxvKGcfQumxqKmdCJCqWV0KY+s3vobLGUV+EDDrF286i0ZulqnHZtAO\nkGGTXsptC86gZHaRYsIuZ0US5KYC1h1tGmwbISdvmfwaXm7N5XW2qU5W3VUZYx4Q\nwjiZ+JJ5RddKYM8TIvK6+AGPqcLdqJxDp/rr5jb0MVJsv+m9BkWRVDSBvnp+zE+b\nJWvUMRc3FeMwqxYAzWVpfsUFUDuc/94fuL6Z68kCgYEA6zuo1Jbe8yEFd8KwJvEj\nN6S7OYmaud9wtKfLTJLa0OmkiXqd88Lw1Mf8uvb4TBP+IOS0XwJYL/R2dmBix4TY\neMesaK1h9twsbKp86sq2bDL5SbT/cWLI6xpgV27vnYIcXKHdtO48FbUrYriTf1sb\n8XYGKAxygcjYewYiLAow6MkCgYEAwFzGDj2HntZaLvHAi/10d7CMhF1bLKNK2l/Q\n46TW+xjAjlAToQ1T+2gfhEvTQ0b+nLJt/ld/FqTogT/3TKpoFG3YWlLZ8ruoPOXY\nxBhL/kxZLchmHN45hzXkd91lOlRLf9TsfHxsYEkOU56kkpIw9zSbHxHbTgUgpwM+\nB5qT8CUCgYB9uuyZfG58O1kl0uy+U8MEGctsjI0j7jbaiJkUO6YzZb5pMR29zaNV\nx/Lgp+K9Hy6EvFlgMuuZ7itnSEtj4zClFeykIpArFzGzf0i3YlQw7unpqJGkNC25\n4+Y8tXHjmUi5hlbvPyrkW2puIMPNnZAI9pGB1G1by1NSJkwbh/LuaQKBgCBB7nyI\n2OtL6seghrdzA0rm8kloFlf/8hd4peDmzZ5B4lh7GS+SupiYN2DKDl1j1GKWkVdr\neMZlVRAHmALlOJrkaLmM1zubOHUt3hHUOTolt3az+luw8Fi6Mtve5pDHffmrzRR7\nEPl8hsiC+/oQReHOkoy9Q9driLQ5GPfRdil5AoGAX64TOs4kpbzM045OgM65Cm2t\nW+SreofkSv2ErT4dzAoPhs0x1xX6BUDNCqn74einRfr1NBJj/llkbMiD8QAHJNq2\npqS4IynQWgf4hYsJlrvG0F79WWadArLVQNLxebGxThzx6l+mIm1wGn/xHibbYBB2\nJz01Ou8GC/69JgjMjas=\n-----END PRIVATE KEY-----\n'; }
function rsaPrivatePkcs1() { return '-----BEGIN RSA PRIVATE KEY-----\nMIIEoAIBAAKCAQEAsMH+DVfu7G1NM8Gi+VdQAcfXqXcOe9X+MDbjXInPMewkXISL\n+A7g5Sqx7COk9MvPqSjHauKx6w3hLXnk8pa4Cux56ig3t9W/Asa+wZAy5Wp3RJj9\n/1T0LuxBQze9KxevCHTJ/8vcsG6cGDRDQiMo8kZH62jOS6+bAVSo35mvCn7tT3aB\nN4jJiV9s4UF0I5ZwlQBL3pWX0kYeuE5gb/F6Ad3afdx0JkZb6bhDm1YuqdMFYjGL\nnkmmBb4qiTjbZ8LvcUVQYl14Pais+rA7EcAbjUE4JgqQF7IMHCpiI5dabxAK6JZ/\ni0wdxy9tGiCb0w9XaCeFqtzf9e2eaYuI6LkVDQIDAQABAoH/QpTQdSqklTBRJG3S\nHsjWYDJsykBOl0t+nROEP+jX17ZEnwll9eIB9/Po10p+r864+9agjWKdCxZjthTo\nxgQi/fpnTyQokwWCXI8vV2jc2ms6dlyICxSd5mwMbyhnH0LpsaipnQiQqlldCmPr\nN76GyxlFfhAw6xdvOotGbpapx2bQDpBhk17KbQvOoGR2kWLCLmdFEuSmAtYdbRps\nGyEnb5n8Gl5uzeV1tqlOVt1VGWMeEMI4mfiSeUXXSmDPEyLyuvgBj6nC3aicQ6f6\n6+Y29DFSbL/pvQZFkVQ0gb56fsxPmyVr1DEXNxXjMKsWAM1laX7FBVA7nP/eH7i+\nmevJAoGBAOs7qNSW3vMhBXfCsCbxIzekuzmJmrnfcLSny0yS2tDppIl6nfPC8NTH\n/Lr2+EwT/iDktF8CWC/0dnZgYseE2HjHrGitYfbcLGyqfOrKtmwy+Um0/3FiyOsa\nYFdu752CHFyh3bTuPBW1K2K4k39bG/F2BigMcoHI2HsGIiwKMOjJAoGBAMBcxg49\nh57WWi7xwIv9dHewjIRdWyyjStpf0OOk1vsYwI5QE6ENU/toH4RL00NG/pyybf5X\nfxak6IE/90yqaBRt2FpS2fK7qDzl2MQYS/5MWS3IZhzeOYc15HfdZTpUS3/U7Hx8\nbGBJDlOepJKSMPc0mx8R204FIKcDPgeak/AlAoGAfbrsmXxufDtZJdLsvlPDBBnL\nbIyNI+422oiZFDumM2W+aTEdvc2jVcfy4KfivR8uhLxZYDLrme4rZ0hLY+MwpRXs\npCKQKxcxs39It2JUMO7p6aiRpDQtuePmPLVx45lIuYZW7z8q5FtqbiDDzZ2QCPaR\ngdRtW8tTUiZMG4fy7mkCgYAgQe58iNjrS+rHoIa3cwNK5vJJaBZX//IXeKXg5s2e\nQeJYexkvkrqYmDdgyg5dY9RilpFXa3jGZVUQB5gC5Tia5Gi5jNc7mzh1Ld4R1Dk6\nJbd2s/pbsPBYujLb3uaQx335q80UexD5fIbIgvv6EEXhzpKMvUPXa4i0ORj30XYp\neQKBgF+uEzrOJKW8zNOOToDOuQptrVvkq3qH5Er9hK0+HcwKD4bNMdcV+gVAzQqp\n++Hop0X69TQSY/5ZZGzIg/EAByTatqakuCMp0FoH+IWLCZa7xtBe/VlmnQKy1UDS\n8XmxsU4c8epfpiJtcBp/8R4m22AQdic9NTrvBgv+vSYIzI2r\n-----END RSA PRIVATE KEY-----\n'; }
function rsaPublicKey() { return '-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsMH+DVfu7G1NM8Gi+VdQ\nAcfXqXcOe9X+MDbjXInPMewkXISL+A7g5Sqx7COk9MvPqSjHauKx6w3hLXnk8pa4\nCux56ig3t9W/Asa+wZAy5Wp3RJj9/1T0LuxBQze9KxevCHTJ/8vcsG6cGDRDQiMo\n8kZH62jOS6+bAVSo35mvCn7tT3aBN4jJiV9s4UF0I5ZwlQBL3pWX0kYeuE5gb/F6\nAd3afdx0JkZb6bhDm1YuqdMFYjGLnkmmBb4qiTjbZ8LvcUVQYl14Pais+rA7EcAb\njUE4JgqQF7IMHCpiI5dabxAK6JZ/i0wdxy9tGiCb0w9XaCeFqtzf9e2eaYuI6LkV\nDQIDAQAB\n-----END PUBLIC KEY-----\n'; }
function ecPrivatePkcs8() { return '-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgHqV2FCeyxUa9ivDr\nlBxTfnDLAsoI5yqk63+Gu/wHFFehRANCAARuk+iU6lIfCrZ55skHVPDCLJokBINo\nHetn8HlbmnNJwQvHw0cTM3BelUyYVXZkLPW5kCLhhNiRpRgGOm+3vLOI\n-----END PRIVATE KEY-----\n'; }
function ecPrivateSec1() { return '-----BEGIN EC PRIVATE KEY-----\nMHcCAQEEIB6ldhQnssVGvYrw65QcU35wywLKCOcqpOt/hrv8BxRXoAoGCCqGSM49\nAwEHoUQDQgAEbpPolOpSHwq2eebJB1TwwiyaJASDaB3rZ/B5W5pzScELx8NHEzNw\nXpVMmFV2ZCz1uZAi4YTYkaUYBjpvt7yziA==\n-----END EC PRIVATE KEY-----\n'; }
function ecPublicKey() { return '-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEbpPolOpSHwq2eebJB1TwwiyaJASD\naB3rZ/B5W5pzScELx8NHEzNwXpVMmFV2ZCz1uZAi4YTYkaUYBjpvt7yziA==\n-----END PUBLIC KEY-----\n'; }
function rsaRoundTrip() {
const sig = __thaw_crypto_module.createSign('RSA-SHA256').update('hello world').sign(rsaPrivatePkcs8(), 'hex');
const ok = __thaw_crypto_module.createVerify('RSA-SHA256').update('hello world').verify(rsaPublicKey(), sig, 'hex');
const okPkcs1Key = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(rsaPublicKey(), __thaw_crypto_module.createSign('sha256').update('hello world').sign(rsaPrivatePkcs1(), 'hex'), 'hex');
const tamperedMessage = __thaw_crypto_module.createVerify('sha256').update('goodbye world').verify(rsaPublicKey(), sig, 'hex');
const tamperedSignature = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(rsaPublicKey(), sig.slice(0, -2) + '00', 'hex');
const opensslSig = 'a7e86b1a31fe823a693f5fedd193ebfb60098de3e41bac389e9e52215d785ee5cc3d211560be5dd87cbadbc264bdea375f96e87420f71b30891352d9a7631e2e1d42d0f8422a4f9af640b3a41617d754e7bdc1738436ac81fa79f6455ed431ab9221671d9582687bf3999ecf618085426bcc3a41c69ec89d0aedcbfaa835d37a89e215859d85d4509afbfb22b569318e779193764f63b9dabfb11c5360b08c2bfcfe257ab5f08f1438f397b2bf284f46848c8ff660ead19e35c67727fef275baf9d91fe56f7a9544f04dcc1ed1152bf737c17a141aaca770fdf4241cef76141722d8910a178b8ca3f183502d49a0c8cf51443e2a797b4ee6ad2be49aea9c9e40';
const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(rsaPublicKey(), opensslSig, 'hex');
return [ok, okPkcs1Key, tamperedMessage, tamperedSignature, opensslVerifies];
}
function rsaPssRoundTrip() {
const sig = __thaw_crypto_module.createSign('sha256').update('hello world').sign({ key: rsaPrivatePkcs8(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PSS_PADDING }, 'hex');
const ok = __thaw_crypto_module.createVerify('sha256').update('hello world').verify({ key: rsaPublicKey(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PSS_PADDING }, sig, 'hex');
const opensslSig = '37ceb5072ae254254115b5910f7a8e45723f1a6ca203eb2084074ab4f95672a0c9d6db585f5bed06cef0208bc5ed474ae0177422928739b05eb40ea4dc2da6204bb73d3e88174f32e01cb94b21b317615e61b8df7e561303261f0868ff704013b20f976c8a77ab5a802f15348b0969a35e57802cd7068f7a386481831c43f19bb511882e5163368ef25f93f73bf47f4374483772dddbb8ca2c03defdb5c4fca89549fea202fab7bf5ca82fc55953119781219ff67b58689866466f2c2bddb4b2f152244b8562fd5e13e3e5e087137e5835cd53978b10f4d4dd37e57ee4aff83dafc1973b0600a780938bddc559bd57b46cdc099895fbd284963a67fe0e9d120b';
const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify({ key: rsaPublicKey(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PSS_PADDING }, opensslSig, 'hex');
return [ok, opensslVerifies];
}
function ecdsaRoundTrip() {
const sig = __thaw_crypto_module.createSign('sha256').update('hello world').sign(ecPrivatePkcs8(), 'hex');
const ok = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(ecPublicKey(), sig, 'hex');
const okSec1Key = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(ecPublicKey(), __thaw_crypto_module.createSign('sha256').update('hello world').sign(ecPrivateSec1(), 'hex'), 'hex');
const tamperedMessage = __thaw_crypto_module.createVerify('sha256').update('goodbye world').verify(ecPublicKey(), sig, 'hex');
const opensslSig = '304502210091b5f1192a836c1d9582197cab6ceac74a952a794d1276781e8f1ac1cf9a001b02203c65bd65ec0b91e24d1f5b52ccaa466e28748e2e56b944d5e7a9aec1bc712210';
const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(ecPublicKey(), opensslSig, 'hex');
return [ok, okSec1Key, tamperedMessage, opensslVerifies];
}
function oneShotSignVerify() {
const sig = __thaw_crypto_module.sign('sha256', 'one shot', rsaPrivatePkcs8());
return __thaw_crypto_module.verify('sha256', 'one shot', rsaPublicKey(), sig);
}
function keyObjectAsymmetricInfo() {
const priv = __thaw_crypto_module.createPrivateKey(rsaPrivatePkcs8());
const pub = __thaw_crypto_module.createPublicKey(rsaPublicKey());
const ecPub = __thaw_crypto_module.createPublicKey(ecPublicKey());
return [priv.type, priv.asymmetricKeyType, pub.type, pub.asymmetricKeyType, ecPub.asymmetricKeyType, ecPub.asymmetricKeyDetails.namedCurve];
}"##,
        r##"console.log(JSON.stringify(ecdsaRoundTrip()));"##,
    ]
    .concat();
    assert_eq!(run(&program), r#"[true,true,false,true]"#);
}

#[test]
fn asym_p521_and_ed25519_sign_verify_match_openssl() {
    let program = [
        r##"function ec521PrivateKey() { return '-----BEGIN PRIVATE KEY-----\nMIHuAgEAMBAGByqGSM49AgEGBSuBBAAjBIHWMIHTAgEBBEIBjSW4dVBkRUfwm4RF\nl+sHT3Bb+K3g3OK8rBc9FKUI7yseNOqjMJH4NWAA4COGQLrcrEZd8i3mC6PMiYlP\nW2kM0rWhgYkDgYYABABj3B3pln7Wni4OZxl1rPfCXviip4IG30MzhbXxCmsdtWZW\n9rs74AS0HzfD3JmxhpsyykdhymyRcCUTK6Dg0AMn4AFkkRuVJ6EH5naQbrW/gm1S\nVhMwsFhDH8U4eFloVh5F224ipvr8StOkFbX7bc8SEAInT4mJoPOVhHIh3Nic5RPT\nzw==\n-----END PRIVATE KEY-----\n'; }
function ec521PublicKey() { return '-----BEGIN PUBLIC KEY-----\nMIGbMBAGByqGSM49AgEGBSuBBAAjA4GGAAQAY9wd6ZZ+1p4uDmcZdaz3wl74oqeC\nBt9DM4W18QprHbVmVva7O+AEtB83w9yZsYabMspHYcpskXAlEyug4NADJ+ABZJEb\nlSehB+Z2kG61v4JtUlYTMLBYQx/FOHhZaFYeRdtuIqb6/ErTpBW1+23PEhACJ0+J\niaDzlYRyIdzYnOUT088=\n-----END PUBLIC KEY-----\n'; }
function ed25519PrivateKey() { return '-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIIQeFoclOMRan6+SUSRbRw78/TZyhWbyoWJcEbyE9MM+\n-----END PRIVATE KEY-----\n'; }
function ed25519PublicKey() { return '-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEAbKI14Nm8FASaqC+aL64yTxj+nIFlCG8Ylhs1gQwQDls=\n-----END PUBLIC KEY-----\n'; }
function ec521RoundTrip() {
const sig = __thaw_crypto_module.createSign('sha512').update('hello world').sign(ec521PrivateKey(), 'hex');
const ok = __thaw_crypto_module.createVerify('sha512').update('hello world').verify(ec521PublicKey(), sig, 'hex');
const tamperedMessage = __thaw_crypto_module.createVerify('sha512').update('goodbye world').verify(ec521PublicKey(), sig, 'hex');
const opensslSig = '30818702414b69e53677eca1ee5f0d40de0ad064064a480a1de9b0b9fe82e898778f705e56f57b8899e53b58d32603aee99f18c4921330487c339e1b3c8d05c0011c90122c1b02420091998e2df97d18f040bcca159eae7f27f3d046188801b92c88771e0dbd2fe74767b20bde678eb460f1819e30523b5c9f5ba1580b74a495acdf67b83009728d3225';
const opensslVerifies = __thaw_crypto_module.createVerify('sha512').update('hello world').verify(ec521PublicKey(), opensslSig, 'hex');
return [ok, tamperedMessage, opensslVerifies];
}
function ed25519RoundTrip() {
const sig = __thaw_crypto_module.sign('sha256', 'hello world', ed25519PrivateKey());
const ok = __thaw_crypto_module.verify('sha256', 'hello world', ed25519PublicKey(), sig);
const tamperedMessage = __thaw_crypto_module.verify('sha256', 'goodbye world', ed25519PublicKey(), sig);
const opensslSig = Buffer.from('34eb966388de794ba39493a2da457c263e716beea079da49cb8ce8cd544dfaf1bc494987096177c3edda654fe1fbb1efe34e8266eec20c2e17a69481ddc1780d', 'hex');
const opensslVerifies = __thaw_crypto_module.verify('sha256', 'hello world', ed25519PublicKey(), opensslSig);
return [ok, tamperedMessage, opensslVerifies];
}
function keyObjectAsymmetricInfoP521AndEd25519() {
const ecPriv = __thaw_crypto_module.createPrivateKey(ec521PrivateKey());
const edPub = __thaw_crypto_module.createPublicKey(ed25519PublicKey());
return [ecPriv.asymmetricKeyType, ecPriv.asymmetricKeyDetails.namedCurve, edPub.asymmetricKeyType];
}"##,
        r##"
console.log(JSON.stringify([ec521RoundTrip(), ed25519RoundTrip(), keyObjectAsymmetricInfoP521AndEd25519()]));
"##,
    ]
    .concat();
    assert_eq!(
        run(&program),
        r#"[[true,false,true],[true,false,true],["ec","secp521r1","ed25519"]]"#
    );
}

#[test]
fn asym_ed448_sign_verify_matches_openssl() {
    let program = [
        r##"function ed448PrivateKey() { return '-----BEGIN PRIVATE KEY-----\nMEcCAQAwBQYDK2VxBDsEOWI45zIeyrnS8uNzaeXrUST94qjNL9kNXt+kgTRCpDQ+\n2z9MswYirqpG3vBlGeJPDeOTwUTjRfgBpg==\n-----END PRIVATE KEY-----\n'; }
function ed448PublicKey() { return '-----BEGIN PUBLIC KEY-----\nMEMwBQYDK2VxAzoAsFWCVQcqbRnP1fEgvOSUY+ox4UCiTUB7BBkIi5qa45sFbG+S\n9Q7VuaLnCrJEg0kZKGFQZNwh2nKA\n-----END PUBLIC KEY-----\n'; }
function ed448RoundTrip() {
const message = 'hello ed448 signature test';
const sig = __thaw_crypto_module.sign('sha256', message, ed448PrivateKey());
const ok = __thaw_crypto_module.verify('sha256', message, ed448PublicKey(), sig);
const tamperedMessage = __thaw_crypto_module.verify('sha256', 'goodbye world', ed448PublicKey(), sig);
const opensslSig = Buffer.from('33e1730abda15ccd9421c9692972486b8f5d276ad88c589e55de128c4821a7faa414bd58da8426a9bd4a953387e4277358715f1e7ea2737e80754233aa485b36a9b52dfc405f8c43d6d95cc2e48d90b74c8bd68217410b1be081c7022de70ddf131eea49f012ddf2d7629aad2c4b73761d00', 'hex');
const opensslVerifies = __thaw_crypto_module.verify('sha256', message, ed448PublicKey(), opensslSig);
return [ok, tamperedMessage, opensslVerifies];
}
function keyObjectAsymmetricInfoEd448() {
const priv = __thaw_crypto_module.createPrivateKey(ed448PrivateKey());
const pub = __thaw_crypto_module.createPublicKey(ed448PublicKey());
return [priv.asymmetricKeyType, pub.asymmetricKeyType];
}"##,
        r##"
console.log(JSON.stringify([ed448RoundTrip(), keyObjectAsymmetricInfoEd448()]));
"##,
    ]
    .concat();
    assert_eq!(run(&program), r#"[[true,false,true],["ed448","ed448"]]"#);
}

#[test]
fn asym_rsa_pkcs1_and_pss_sign_verify_match_openssl() {
    let program = [
        r##"function rsaPrivatePkcs8() { return '-----BEGIN PRIVATE KEY-----\nMIIEugIBADANBgkqhkiG9w0BAQEFAASCBKQwggSgAgEAAoIBAQCwwf4NV+7sbU0z\nwaL5V1ABx9epdw571f4wNuNcic8x7CRchIv4DuDlKrHsI6T0y8+pKMdq4rHrDeEt\neeTylrgK7HnqKDe31b8Cxr7BkDLlandEmP3/VPQu7EFDN70rF68IdMn/y9ywbpwY\nNENCIyjyRkfraM5Lr5sBVKjfma8Kfu1PdoE3iMmJX2zhQXQjlnCVAEvelZfSRh64\nTmBv8XoB3dp93HQmRlvpuEObVi6p0wViMYueSaYFviqJONtnwu9xRVBiXXg9qKz6\nsDsRwBuNQTgmCpAXsgwcKmIjl1pvEAroln+LTB3HL20aIJvTD1doJ4Wq3N/17Z5p\ni4jouRUNAgMBAAECgf9ClNB1KqSVMFEkbdIeyNZgMmzKQE6XS36dE4Q/6NfXtkSf\nCWX14gH38+jXSn6vzrj71qCNYp0LFmO2FOjGBCL9+mdPJCiTBYJcjy9XaNzaazp2\nXIgLFJ3mbAxvKGcfQumxqKmdCJCqWV0KY+s3vobLGUV+EDDrF286i0ZulqnHZtAO\nkGGTXsptC86gZHaRYsIuZ0US5KYC1h1tGmwbISdvmfwaXm7N5XW2qU5W3VUZYx4Q\nwjiZ+JJ5RddKYM8TIvK6+AGPqcLdqJxDp/rr5jb0MVJsv+m9BkWRVDSBvnp+zE+b\nJWvUMRc3FeMwqxYAzWVpfsUFUDuc/94fuL6Z68kCgYEA6zuo1Jbe8yEFd8KwJvEj\nN6S7OYmaud9wtKfLTJLa0OmkiXqd88Lw1Mf8uvb4TBP+IOS0XwJYL/R2dmBix4TY\neMesaK1h9twsbKp86sq2bDL5SbT/cWLI6xpgV27vnYIcXKHdtO48FbUrYriTf1sb\n8XYGKAxygcjYewYiLAow6MkCgYEAwFzGDj2HntZaLvHAi/10d7CMhF1bLKNK2l/Q\n46TW+xjAjlAToQ1T+2gfhEvTQ0b+nLJt/ld/FqTogT/3TKpoFG3YWlLZ8ruoPOXY\nxBhL/kxZLchmHN45hzXkd91lOlRLf9TsfHxsYEkOU56kkpIw9zSbHxHbTgUgpwM+\nB5qT8CUCgYB9uuyZfG58O1kl0uy+U8MEGctsjI0j7jbaiJkUO6YzZb5pMR29zaNV\nx/Lgp+K9Hy6EvFlgMuuZ7itnSEtj4zClFeykIpArFzGzf0i3YlQw7unpqJGkNC25\n4+Y8tXHjmUi5hlbvPyrkW2puIMPNnZAI9pGB1G1by1NSJkwbh/LuaQKBgCBB7nyI\n2OtL6seghrdzA0rm8kloFlf/8hd4peDmzZ5B4lh7GS+SupiYN2DKDl1j1GKWkVdr\neMZlVRAHmALlOJrkaLmM1zubOHUt3hHUOTolt3az+luw8Fi6Mtve5pDHffmrzRR7\nEPl8hsiC+/oQReHOkoy9Q9driLQ5GPfRdil5AoGAX64TOs4kpbzM045OgM65Cm2t\nW+SreofkSv2ErT4dzAoPhs0x1xX6BUDNCqn74einRfr1NBJj/llkbMiD8QAHJNq2\npqS4IynQWgf4hYsJlrvG0F79WWadArLVQNLxebGxThzx6l+mIm1wGn/xHibbYBB2\nJz01Ou8GC/69JgjMjas=\n-----END PRIVATE KEY-----\n'; }
function rsaPrivatePkcs1() { return '-----BEGIN RSA PRIVATE KEY-----\nMIIEoAIBAAKCAQEAsMH+DVfu7G1NM8Gi+VdQAcfXqXcOe9X+MDbjXInPMewkXISL\n+A7g5Sqx7COk9MvPqSjHauKx6w3hLXnk8pa4Cux56ig3t9W/Asa+wZAy5Wp3RJj9\n/1T0LuxBQze9KxevCHTJ/8vcsG6cGDRDQiMo8kZH62jOS6+bAVSo35mvCn7tT3aB\nN4jJiV9s4UF0I5ZwlQBL3pWX0kYeuE5gb/F6Ad3afdx0JkZb6bhDm1YuqdMFYjGL\nnkmmBb4qiTjbZ8LvcUVQYl14Pais+rA7EcAbjUE4JgqQF7IMHCpiI5dabxAK6JZ/\ni0wdxy9tGiCb0w9XaCeFqtzf9e2eaYuI6LkVDQIDAQABAoH/QpTQdSqklTBRJG3S\nHsjWYDJsykBOl0t+nROEP+jX17ZEnwll9eIB9/Po10p+r864+9agjWKdCxZjthTo\nxgQi/fpnTyQokwWCXI8vV2jc2ms6dlyICxSd5mwMbyhnH0LpsaipnQiQqlldCmPr\nN76GyxlFfhAw6xdvOotGbpapx2bQDpBhk17KbQvOoGR2kWLCLmdFEuSmAtYdbRps\nGyEnb5n8Gl5uzeV1tqlOVt1VGWMeEMI4mfiSeUXXSmDPEyLyuvgBj6nC3aicQ6f6\n6+Y29DFSbL/pvQZFkVQ0gb56fsxPmyVr1DEXNxXjMKsWAM1laX7FBVA7nP/eH7i+\nmevJAoGBAOs7qNSW3vMhBXfCsCbxIzekuzmJmrnfcLSny0yS2tDppIl6nfPC8NTH\n/Lr2+EwT/iDktF8CWC/0dnZgYseE2HjHrGitYfbcLGyqfOrKtmwy+Um0/3FiyOsa\nYFdu752CHFyh3bTuPBW1K2K4k39bG/F2BigMcoHI2HsGIiwKMOjJAoGBAMBcxg49\nh57WWi7xwIv9dHewjIRdWyyjStpf0OOk1vsYwI5QE6ENU/toH4RL00NG/pyybf5X\nfxak6IE/90yqaBRt2FpS2fK7qDzl2MQYS/5MWS3IZhzeOYc15HfdZTpUS3/U7Hx8\nbGBJDlOepJKSMPc0mx8R204FIKcDPgeak/AlAoGAfbrsmXxufDtZJdLsvlPDBBnL\nbIyNI+422oiZFDumM2W+aTEdvc2jVcfy4KfivR8uhLxZYDLrme4rZ0hLY+MwpRXs\npCKQKxcxs39It2JUMO7p6aiRpDQtuePmPLVx45lIuYZW7z8q5FtqbiDDzZ2QCPaR\ngdRtW8tTUiZMG4fy7mkCgYAgQe58iNjrS+rHoIa3cwNK5vJJaBZX//IXeKXg5s2e\nQeJYexkvkrqYmDdgyg5dY9RilpFXa3jGZVUQB5gC5Tia5Gi5jNc7mzh1Ld4R1Dk6\nJbd2s/pbsPBYujLb3uaQx335q80UexD5fIbIgvv6EEXhzpKMvUPXa4i0ORj30XYp\neQKBgF+uEzrOJKW8zNOOToDOuQptrVvkq3qH5Er9hK0+HcwKD4bNMdcV+gVAzQqp\n++Hop0X69TQSY/5ZZGzIg/EAByTatqakuCMp0FoH+IWLCZa7xtBe/VlmnQKy1UDS\n8XmxsU4c8epfpiJtcBp/8R4m22AQdic9NTrvBgv+vSYIzI2r\n-----END RSA PRIVATE KEY-----\n'; }
function rsaPublicKey() { return '-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsMH+DVfu7G1NM8Gi+VdQ\nAcfXqXcOe9X+MDbjXInPMewkXISL+A7g5Sqx7COk9MvPqSjHauKx6w3hLXnk8pa4\nCux56ig3t9W/Asa+wZAy5Wp3RJj9/1T0LuxBQze9KxevCHTJ/8vcsG6cGDRDQiMo\n8kZH62jOS6+bAVSo35mvCn7tT3aBN4jJiV9s4UF0I5ZwlQBL3pWX0kYeuE5gb/F6\nAd3afdx0JkZb6bhDm1YuqdMFYjGLnkmmBb4qiTjbZ8LvcUVQYl14Pais+rA7EcAb\njUE4JgqQF7IMHCpiI5dabxAK6JZ/i0wdxy9tGiCb0w9XaCeFqtzf9e2eaYuI6LkV\nDQIDAQAB\n-----END PUBLIC KEY-----\n'; }
function ecPrivatePkcs8() { return '-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgHqV2FCeyxUa9ivDr\nlBxTfnDLAsoI5yqk63+Gu/wHFFehRANCAARuk+iU6lIfCrZ55skHVPDCLJokBINo\nHetn8HlbmnNJwQvHw0cTM3BelUyYVXZkLPW5kCLhhNiRpRgGOm+3vLOI\n-----END PRIVATE KEY-----\n'; }
function ecPrivateSec1() { return '-----BEGIN EC PRIVATE KEY-----\nMHcCAQEEIB6ldhQnssVGvYrw65QcU35wywLKCOcqpOt/hrv8BxRXoAoGCCqGSM49\nAwEHoUQDQgAEbpPolOpSHwq2eebJB1TwwiyaJASDaB3rZ/B5W5pzScELx8NHEzNw\nXpVMmFV2ZCz1uZAi4YTYkaUYBjpvt7yziA==\n-----END EC PRIVATE KEY-----\n'; }
function ecPublicKey() { return '-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEbpPolOpSHwq2eebJB1TwwiyaJASD\naB3rZ/B5W5pzScELx8NHEzNwXpVMmFV2ZCz1uZAi4YTYkaUYBjpvt7yziA==\n-----END PUBLIC KEY-----\n'; }
function rsaRoundTrip() {
const sig = __thaw_crypto_module.createSign('RSA-SHA256').update('hello world').sign(rsaPrivatePkcs8(), 'hex');
const ok = __thaw_crypto_module.createVerify('RSA-SHA256').update('hello world').verify(rsaPublicKey(), sig, 'hex');
const okPkcs1Key = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(rsaPublicKey(), __thaw_crypto_module.createSign('sha256').update('hello world').sign(rsaPrivatePkcs1(), 'hex'), 'hex');
const tamperedMessage = __thaw_crypto_module.createVerify('sha256').update('goodbye world').verify(rsaPublicKey(), sig, 'hex');
const tamperedSignature = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(rsaPublicKey(), sig.slice(0, -2) + '00', 'hex');
const opensslSig = 'a7e86b1a31fe823a693f5fedd193ebfb60098de3e41bac389e9e52215d785ee5cc3d211560be5dd87cbadbc264bdea375f96e87420f71b30891352d9a7631e2e1d42d0f8422a4f9af640b3a41617d754e7bdc1738436ac81fa79f6455ed431ab9221671d9582687bf3999ecf618085426bcc3a41c69ec89d0aedcbfaa835d37a89e215859d85d4509afbfb22b569318e779193764f63b9dabfb11c5360b08c2bfcfe257ab5f08f1438f397b2bf284f46848c8ff660ead19e35c67727fef275baf9d91fe56f7a9544f04dcc1ed1152bf737c17a141aaca770fdf4241cef76141722d8910a178b8ca3f183502d49a0c8cf51443e2a797b4ee6ad2be49aea9c9e40';
const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(rsaPublicKey(), opensslSig, 'hex');
return [ok, okPkcs1Key, tamperedMessage, tamperedSignature, opensslVerifies];
}
function rsaPssRoundTrip() {
const sig = __thaw_crypto_module.createSign('sha256').update('hello world').sign({ key: rsaPrivatePkcs8(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PSS_PADDING }, 'hex');
const ok = __thaw_crypto_module.createVerify('sha256').update('hello world').verify({ key: rsaPublicKey(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PSS_PADDING }, sig, 'hex');
const opensslSig = '37ceb5072ae254254115b5910f7a8e45723f1a6ca203eb2084074ab4f95672a0c9d6db585f5bed06cef0208bc5ed474ae0177422928739b05eb40ea4dc2da6204bb73d3e88174f32e01cb94b21b317615e61b8df7e561303261f0868ff704013b20f976c8a77ab5a802f15348b0969a35e57802cd7068f7a386481831c43f19bb511882e5163368ef25f93f73bf47f4374483772dddbb8ca2c03defdb5c4fca89549fea202fab7bf5ca82fc55953119781219ff67b58689866466f2c2bddb4b2f152244b8562fd5e13e3e5e087137e5835cd53978b10f4d4dd37e57ee4aff83dafc1973b0600a780938bddc559bd57b46cdc099895fbd284963a67fe0e9d120b';
const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify({ key: rsaPublicKey(), padding: __thaw_crypto_module.constants.RSA_PKCS1_PSS_PADDING }, opensslSig, 'hex');
return [ok, opensslVerifies];
}
function ecdsaRoundTrip() {
const sig = __thaw_crypto_module.createSign('sha256').update('hello world').sign(ecPrivatePkcs8(), 'hex');
const ok = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(ecPublicKey(), sig, 'hex');
const okSec1Key = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(ecPublicKey(), __thaw_crypto_module.createSign('sha256').update('hello world').sign(ecPrivateSec1(), 'hex'), 'hex');
const tamperedMessage = __thaw_crypto_module.createVerify('sha256').update('goodbye world').verify(ecPublicKey(), sig, 'hex');
const opensslSig = '304502210091b5f1192a836c1d9582197cab6ceac74a952a794d1276781e8f1ac1cf9a001b02203c65bd65ec0b91e24d1f5b52ccaa466e28748e2e56b944d5e7a9aec1bc712210';
const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(ecPublicKey(), opensslSig, 'hex');
return [ok, okSec1Key, tamperedMessage, opensslVerifies];
}
function oneShotSignVerify() {
const sig = __thaw_crypto_module.sign('sha256', 'one shot', rsaPrivatePkcs8());
return __thaw_crypto_module.verify('sha256', 'one shot', rsaPublicKey(), sig);
}
function keyObjectAsymmetricInfo() {
const priv = __thaw_crypto_module.createPrivateKey(rsaPrivatePkcs8());
const pub = __thaw_crypto_module.createPublicKey(rsaPublicKey());
const ecPub = __thaw_crypto_module.createPublicKey(ecPublicKey());
return [priv.type, priv.asymmetricKeyType, pub.type, pub.asymmetricKeyType, ecPub.asymmetricKeyType, ecPub.asymmetricKeyDetails.namedCurve];
}"##,
        r##"
var m = __thaw_crypto_module;
var PSS = m.constants.RSA_PKCS1_PSS_PADDING;
var sig = m.createSign('RSA-SHA256').update('hello world').sign(rsaPrivatePkcs1(), 'hex');
var ok = m.createVerify('sha256').update('hello world').verify(rsaPublicKey(), sig, 'hex');
var tamperedMessage = m.createVerify('sha256').update('goodbye world').verify(rsaPublicKey(), sig, 'hex');
var flipped = sig.slice(0, -2) + (sig.slice(-2) === '00' ? '01' : '00');
var tamperedSignature = m.createVerify('sha256').update('hello world').verify(rsaPublicKey(), flipped, 'hex');
var opensslRs = m.createVerify('sha256').update('hello world').verify(rsaPublicKey(), 'a7e86b1a31fe823a693f5fedd193ebfb60098de3e41bac389e9e52215d785ee5cc3d211560be5dd87cbadbc264bdea375f96e87420f71b30891352d9a7631e2e1d42d0f8422a4f9af640b3a41617d754e7bdc1738436ac81fa79f6455ed431ab9221671d9582687bf3999ecf618085426bcc3a41c69ec89d0aedcbfaa835d37a89e215859d85d4509afbfb22b569318e779193764f63b9dabfb11c5360b08c2bfcfe257ab5f08f1438f397b2bf284f46848c8ff660ead19e35c67727fef275baf9d91fe56f7a9544f04dcc1ed1152bf737c17a141aaca770fdf4241cef76141722d8910a178b8ca3f183502d49a0c8cf51443e2a797b4ee6ad2be49aea9c9e40', 'hex');
var pssSig = m.createSign('sha256').update('hello world').sign({ key: rsaPrivatePkcs8(), padding: PSS }, 'hex');
var pssOk = m.createVerify('sha256').update('hello world').verify({ key: rsaPublicKey(), padding: PSS }, pssSig, 'hex');
var pssOpenssl = m.createVerify('sha256').update('hello world').verify({ key: rsaPublicKey(), padding: PSS }, '37ceb5072ae254254115b5910f7a8e45723f1a6ca203eb2084074ab4f95672a0c9d6db585f5bed06cef0208bc5ed474ae0177422928739b05eb40ea4dc2da6204bb73d3e88174f32e01cb94b21b317615e61b8df7e561303261f0868ff704013b20f976c8a77ab5a802f15348b0969a35e57802cd7068f7a386481831c43f19bb511882e5163368ef25f93f73bf47f4374483772dddbb8ca2c03defdb5c4fca89549fea202fab7bf5ca82fc55953119781219ff67b58689866466f2c2bddb4b2f152244b8562fd5e13e3e5e087137e5835cd53978b10f4d4dd37e57ee4aff83dafc1973b0600a780938bddc559bd57b46cdc099895fbd284963a67fe0e9d120b', 'hex');
var pssAsPkcs1 = m.createVerify('sha256').update('hello world').verify(rsaPublicKey(), pssSig, 'hex');
console.log(JSON.stringify([ok, tamperedMessage, tamperedSignature, opensslRs, pssOk, pssOpenssl, pssAsPkcs1]));
"##,
    ]
    .concat();
    assert_eq!(run(&program), r#"[true,false,false,true,true,true,false]"#);
}

#[test]
fn asym_der_and_encrypted_pkcs8_import_match_openssl() {
    let program = [
        r##"function rsaPublicPem() { return '-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAoORKaNK2ZuUTmkCb/Oqh\n3vw+cO8OCUFJRz+nPYP9gYArPbwy8I3nJe2ft5rVHPTn8u4gzSVwQGBpQ5fYMx4g\n80htGj3JD16v1DHg8qIGDnnk9MGGEHJ6DcdJsKiWK8SK2xEttirRTY5fHGQwkAWg\nMDFIjzG6OIoGDCZyRH+1zY1xdP617629Z2UvNL5eB9A6FPHwpmSlgr8T/nHmqGeN\nL6VJuC61pnZTehGm6hvUMdqPEm0ciiHmavx0DAaWqzVjLBb5jTssayMabrmRCUdq\nTpe3g/MeBcdQ/LCwF/j0QKmgoL3gzWubFA8IO+e2M+AnkV25wbrVTV4ysvWlzdKK\nxwIDAQAB\n-----END PUBLIC KEY-----\n'; }
function rsaPrivateDerHex() { return '308204be020100300d06092a864886f70d0101010500048204a8308204a40201000282010100a0e44a68d2b666e5139a409bfceaa1defc3e70ef0e094149473fa73d83fd81802b3dbc32f08de725ed9fb79ad51cf4e7f2ee20cd25704060694397d8331e20f3486d1a3dc90f5eafd431e0f2a2060e79e4f4c18610727a0dc749b0a8962bc48adb112db62ad14d8e5f1c64309005a03031488f31ba388a060c2672447fb5cd8d7174feb5efadbd67652f34be5e07d03a14f1f0a664a582bf13fe71e6a8678d2fa549b82eb5a676537a11a6ea1bd431da8f126d1c8a21e66afc740c0696ab35632c16f98d3b2c6b231a6eb99109476a4e97b783f31e05c750fcb0b017f8f440a9a0a0bde0cd6b9b140f083be7b633e027915db9c1bad54d5e32b2f5a5cdd28ac7020301000102820100061c457b2fad7fc0e97aad437f5a85e54b1d2ffad444a3b71dbe9c2268f5e2ca345a36e094643f48207b3564eafd1b8c079ce5a004f0fb70edee8440d0c82f262e34fe8f2428b246e93f2fb4e754658e5994b618da5d0ea7a14efa279cf472957776728efd974f63bdd6fd331ef527bd4cd1dda65cd532e0c1eb5fe19c1c127f6237172346eb71d7d5a74e5d0bfafbe7bd1de6da4d361c011100cc7881b2b824df573445e8a166938960942ee19ad27fd6e96da35b954341c1896f6e47ae27d6d4e8b10e21bdcc75ac8f2023866920271dd6b4dce35b11db1e68ac0f9c8901617ea9cb2790b6879417a8f85eb1eff48573eb31580c008e37fae17666bf677d9102818100dffe4d703426d74947f8bf6fd8cdfc709276efe554364ebed55bb74c2555d17c80765d9523426fce8d22082c3fdb0fe3b779b2839348caa9866e2a24c643a75f722618e0ab5d3db6ceec1704bb28b4809714b5cc14811e0412adee71c68ae4f098cfc03c71744288394ce6c4a5cd19b0039dc657e1d0e01d92ef55881aa5a4bf02818100b7e1b9c5782fe8f801f13c994244057e527c3d84e38c641e2c8c27cd33702b073a36077bc12824de002856493f0cf8f5fa20a4b641258b594ae878482414e924aa40e68e09146063a7467f4e3a82d4a2b3d2c3e8afa12135066b6979b40b90be457f8ce0f3f8f627bc1d7368f8e25b5b04af0e56107ee6c4e1da53acc9acf3f902818039a35689e8e195c465a0bca22b47d60da1a2b95869b30fd04b56ae7409a76ba07dedf766c90bef795717cac2982be68ad24b9e83fd025e24015397c49ec009f1a58de818e7ffb641b43d4c2f0b7a0df888e7eb5ff866c1328b1bf69f90576d51fc007997141ab684173a92a74782df794b74edf4ef46b064ebca6a57fb83644102818100a5a93bf776bf1b110c96ec745aa9f39509f51a6b75a18eb54c86fc78b765cfae143886e76c6ea1404c3e0af6b452189d6aba2c0a7288c3912f965e7f07dabaeca8620e145a83bc0f2badac95aacb218c6f9b6b9a5f583815907206b5798a8ddd8db94b0f835d814eed004f707c015a3296f6ab60c83dbbe41661decea56726e9028181009557356289cef177e2aeb16a98f4dae588341eeda0dd47565736836a0338ea0c70f87dc077fe2cf75e0529b2c8c4b07a28a3d8052ce434bcf8bf61bcd8d4c0c7015b629af00743c6cf69a1b868df316b5886986fa1c98a4626975a27660efc84e8b2c61b4a57b89bd36af4b11a4828b8baac12bebe1ae990cb59b81591f9442b'; }
function rsaPublicDerHex() { return '30820122300d06092a864886f70d01010105000382010f003082010a0282010100a0e44a68d2b666e5139a409bfceaa1defc3e70ef0e094149473fa73d83fd81802b3dbc32f08de725ed9fb79ad51cf4e7f2ee20cd25704060694397d8331e20f3486d1a3dc90f5eafd431e0f2a2060e79e4f4c18610727a0dc749b0a8962bc48adb112db62ad14d8e5f1c64309005a03031488f31ba388a060c2672447fb5cd8d7174feb5efadbd67652f34be5e07d03a14f1f0a664a582bf13fe71e6a8678d2fa549b82eb5a676537a11a6ea1bd431da8f126d1c8a21e66afc740c0696ab35632c16f98d3b2c6b231a6eb99109476a4e97b783f31e05c750fcb0b017f8f440a9a0a0bde0cd6b9b140f083be7b633e027915db9c1bad54d5e32b2f5a5cdd28ac70203010001'; }
function rsaPrivateEncryptedPem() { return '-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIFNTBfBgkqhkiG9w0BBQ0wUjAxBgkqhkiG9w0BBQwwJAQQvq2l3eEnfnFD3GVd\nhtvb+QICCAAwDAYIKoZIhvcNAgkFADAdBglghkgBZQMEASoEECBT8TeR7sQt2Vmo\nNpQJ+i0EggTQDM28rgAAPQmRjennOGuBEspP8qH2Yv0tWH51YDJ2xu+FswPDs8H2\n7hA+kQRHhR6U+vw4UM31iQofb0qZ+XSRtfYldZ/4qfk0ND5sTuL4wPzktA/BlCY3\n98m7TDW6OCYN+N1lzHn7FzNqmmEDD3uEUptgCkE2FVgN1bfvMBb25+2zXV6nNbEw\nw+nDIQbdtsn+9P9+TN4DAO43l4Ep/wsVulPyiC2JWxMrly/59V+KMatJ4sPuvVEq\na6JXgdllICzUMZz2cR1/vNzAQklZpI3ePZTSGPJNbH+0QvcPgun8OQGkDnTe3Eb0\nQ3wcHbzjfb+w1x5M6l1KKtkQG1fq7tPHsV/4/HFkabxkQV50ZjpMIfwYeZlYo1Tl\nEHSEgqR/GB5BcefaQSwBAzTJ7Jcn4SIWirIplG77+IG1FftdnIKq6Hx0uok9jC97\nKdP9PTJFO/x7Istm4D844IuZ1wPe4cIZ3gTF/URptaDZYnYUQw2X3tH5mwdCO1Sl\ngaQLsVTAhvPzR6pm4iUGHUwFa6vq6gA9AM8mnGXJdHSdcTeaPnOeJcVxtz+/DKma\nI8dXUeECfzmW30jhQuBaYm4WyKTATjXHxuU3GsbhmKJn51LbfGePu7dXxGsaPvnQ\nIfESiysT0BkdXPql7AEmjEG1HVqMxr/cxMW4Lm2YAHL9mqIig+w9lMiD/CCjOrXR\nZIkgPqOyFEdtZJ6pVvAKg/GDiKX9Yv9V8pjxuGqLdcKv2nf8jXtNE8DusvIoCGtz\n12nHHdBQwDKI4wQyqlEeRrgAALa+lbYHtxIlA7RktRjFA5PtFMdy5ZJf5Ft6Esns\nIxEJSI6AVP0OKQaaTaNZpErfHbgFO/5fqMoLjS6XEy0UVX0ykjW7voNMV91GduGS\nJI3HwH7OkjnvPyG81PwjOzdi/VqtzCYmI2zl6LuHemndEOD06DXUa6mztudIs1cM\nN+WaiAAmbX5ppzN+YzAzhTM3bn3W/vN0vkLydKzekCs1hMs8ErwPkNkmdzySg+Zu\nntvR3OOQXcl+7kNRISEr1OVELB3Th78TDeUcsx/JIGhIFpOIjtGeXz6rjusoJ/FJ\n1jmonh830SiRmBvtdOA+uLHXw4Xsx2ucmWSIYpiQMRlZcsPfQBmU75l5bXrSpGtt\ncEPngFVsSBeS5HuyVpU2fsYmmTZ2S3Q6imzg+1zsVnyXYo00V/3cyFIs+DIYB/a0\ni1iKpns68Bj54qCrkxeaD/G1o1aMlQeekw7fRDZ5PxKI/EzIi/7P/3YL7zL8eRqE\nTa/Bq9Z/Xm7nTVf3oUvzJdL4V3nSS1tmruktz0KXr/d1di2u7gtp6YqyR94DZtju\ncrmuEKVdLhtdrYvstI8Rk9fJm7QR0il8FriwBaeti9aJjPPXDEjmeYf0o01wcpfn\nuxtFM4JLtxOIKORU/enB1XSOsAmLG27IpIJfBWauAJmoPBiE8uOmiYb7Hx5DHUx5\nmgWejWMOTCI/9gYu7RfYc5ZY2lO4BVMefpPWAWufag6Nwolti2l2FHLJBjfd2RBR\ndv44+nFunyx1gE+bEiOUr4YbKRfaxvlTeDLbTTuZFX0wV24asByJhEeFFqJI6SA8\nVK13uyQCON9eCknnEQKxkowmiXsgwzUX4pUmB48WpaHArrk8Ynifkxg=\n-----END ENCRYPTED PRIVATE KEY-----\n'; }
function derRoundTrip() {
const priv = __thaw_crypto_module.createPrivateKey({ key: Buffer.from(rsaPrivateDerHex(), 'hex'), format: 'der' });
const pub = __thaw_crypto_module.createPublicKey({ key: Buffer.from(rsaPublicDerHex(), 'hex'), format: 'der' });
const sig = __thaw_crypto_module.createSign('sha256').update('hello world').sign(priv, 'hex');
const ok = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(pub, sig, 'hex');
const opensslSig = '8d357033bbae2065e82bd5cea6be16e4520ed5b5e2513862d0ab4e9817542944202c8dd7b0e738d71618a7cf93b0d75ea3a04022822ad6858446237729883dfa7c4f399472a742cb68fdb80cfd46a1865cd28f4d21a5b8f539ba6ac7ce8fcf67d03fd159bcd6b47f148822eedf83d93d3815c4fffe3943e008c2f68d07c2ef31c5e2ead3cd82bcf2b6164a4bff2e5ea199ad000f29af65d4cb633eff190a1add38cfea512bf6757a4a04ff868fe530973bf66ad0dc69775da151c86d78e174ded32722b3339da059823efc4e2f6645ca814be5bfe3663808c048ef60aec4a34223615819a354e340935684132fdf93641a4047cf623c7dac075357b7f4e6f514';
const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(pub, opensslSig, 'hex');
return [priv.asymmetricKeyType, pub.asymmetricKeyType, ok, opensslVerifies];
}
function passphraseRoundTrip() {
let wrongPassphraseThrew = false;
try { __thaw_crypto_module.createPrivateKey({ key: rsaPrivateEncryptedPem(), passphrase: 'wrong' }); } catch (error) { wrongPassphraseThrew = true; }
const priv = __thaw_crypto_module.createPrivateKey({ key: rsaPrivateEncryptedPem(), passphrase: 'hunter2' });
const pub = __thaw_crypto_module.createPublicKey(rsaPublicPem());
const sig = __thaw_crypto_module.createSign('sha256').update('hello world').sign(priv, 'hex');
const ok = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(pub, sig, 'hex');
const opensslSig = '8d357033bbae2065e82bd5cea6be16e4520ed5b5e2513862d0ab4e9817542944202c8dd7b0e738d71618a7cf93b0d75ea3a04022822ad6858446237729883dfa7c4f399472a742cb68fdb80cfd46a1865cd28f4d21a5b8f539ba6ac7ce8fcf67d03fd159bcd6b47f148822eedf83d93d3815c4fffe3943e008c2f68d07c2ef31c5e2ead3cd82bcf2b6164a4bff2e5ea199ad000f29af65d4cb633eff190a1add38cfea512bf6757a4a04ff868fe530973bf66ad0dc69775da151c86d78e174ded32722b3339da059823efc4e2f6645ca814be5bfe3663808c048ef60aec4a34223615819a354e340935684132fdf93641a4047cf623c7dac075357b7f4e6f514';
const opensslVerifies = __thaw_crypto_module.createVerify('sha256').update('hello world').verify(pub, opensslSig, 'hex');
return [wrongPassphraseThrew, priv.asymmetricKeyType, ok, opensslVerifies];
}"##,
        r##"
var m = __thaw_crypto_module;
var derPriv = m.createPrivateKey({ key: Buffer.from(rsaPrivateDerHex(), 'hex'), format: 'der' });
var derPub = m.createPublicKey({ key: Buffer.from(rsaPublicDerHex(), 'hex'), format: 'der' });
var wrongPassphraseThrew = false;
try { m.createPrivateKey({ key: rsaPrivateEncryptedPem(), passphrase: 'wrong' }); } catch (error) { wrongPassphraseThrew = true; }
var enc = m.createPrivateKey({ key: rsaPrivateEncryptedPem(), passphrase: 'hunter2' });
var opensslVerifies = m.createVerify('sha256').update('hello world').verify(derPub, '8d357033bbae2065e82bd5cea6be16e4520ed5b5e2513862d0ab4e9817542944202c8dd7b0e738d71618a7cf93b0d75ea3a04022822ad6858446237729883dfa7c4f399472a742cb68fdb80cfd46a1865cd28f4d21a5b8f539ba6ac7ce8fcf67d03fd159bcd6b47f148822eedf83d93d3815c4fffe3943e008c2f68d07c2ef31c5e2ead3cd82bcf2b6164a4bff2e5ea199ad000f29af65d4cb633eff190a1add38cfea512bf6757a4a04ff868fe530973bf66ad0dc69775da151c86d78e174ded32722b3339da059823efc4e2f6645ca814be5bfe3663808c048ef60aec4a34223615819a354e340935684132fdf93641a4047cf623c7dac075357b7f4e6f514', 'hex');
console.log(JSON.stringify([derPriv.asymmetricKeyType, derPub.asymmetricKeyType, derPub.export() === rsaPublicPem(),
  opensslVerifies, wrongPassphraseThrew, enc.asymmetricKeyType, enc.export().indexOf('-----BEGIN PRIVATE KEY-----') === 0]));
"##,
    ]
    .concat();
    assert_eq!(run(&program), r#"["rsa","rsa",true,true,true,"rsa",true]"#);
}

#[test]
fn asym_legacy_encrypted_pem_import_matches_node() {
    let program = [
        r##"function rsaPublicPem() { return '-----BEGIN PUBLIC KEY-----\nMIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAsFF0EaFZ+6scQKkIVSoa\nf8oy5gXO8TDhBbKh7SK0P8ALHEB4flyKVaL4Qvzbh5aRrfpOLwCmlo9ucr4V95SH\nOX7dZ5eo2fONA5CNIaMNY5NzebjV8+kYj4ICRTLugR654Bf22EqCOtMsy4EJ15Ce\nuNfY6+N54bj8pPDyO81NEwn3bRdZr6Wn3Nut6mRG30j/etwNGSFxXoChUAhCbcfL\nxdGzEdIOpL3qBLFnJ2cmurC02wVEx6F1eJt2G6fedxoAgogCtDDtMa89FmeKNh11\nCVxjc6Ma/vGh3jzO5zZegP5yuPGm9mKGsvmNYm6GHCQJUhUmdNNJx0hqDP+gC3X7\nXQIDAQAB\n-----END PUBLIC KEY-----\n'; }
function rsaPrivateDes3Pem() { return '-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: DES-EDE3-CBC,B000B3DC7910EE4F\n\nERnhRI/uyDWAr0JB9jmi8KtmYYV7cUFvVYAIIv7iUSeH3o18T+jLHKvxb5rQDXKy\nSj+sjFWzf/+DaJo/6SXp81RzS3hvIHj0TG5JzGZT8nPk18JOIXB5BUQ6CbFBtfON\nE+wPTU4+cwUlL0ugbbHiKx7tdrtuQT5fUjvWhpZOER+fKyi7uCUyrpZyhPGvlsMA\n2rEv5DfWk494rCnHUg+da2mFT0PKtPGDPUBxIcNwlfwNZERMuwlFYiVTKIclncAn\n0K1PjIskNixOG3SkYVhp8YOKfVar/sFaU7DzPQTXqGJIGOhHQeT9ILUcY4Yb17jx\ngPMtH7gw8j5B68F/r4hl+KIQXSg05G1SkGXKM2zvWu1UUIpW2LlYlD5D4UxtiCit\nNSeV5xVpv4wZZzDTLZAZjRS0SUZ8plY16k9SNQdfTCB+/biFxI/kIHv/hQqB8+ML\nTe8W0rzxDRIwYjc0MB0nhjsfnb+dT6EiNSW4jZEWS5Z5G88CUVjFNI2UOFjwlIO0\nOd0e6d0vuMPJ9rRQMGKA0TfPyioYkJUQ0ED+13ML1W9wC+MvkG7SjtQKLJDQzRMk\nyYGHiToYI9DktYTGipbB2YEt5I/07YWKgEa++dkGYsQZOqFoOGvOvdwZeo2laZjG\nAAp2Q1F2ji5wQR5mUZOtqBWjFJc0VRYA3S26wwqaOk1axydFt6ilk/E1BeblJNzN\nxfmtsg1ZQJof3YTYtlkn2ur/3PWsmerip8H5TgyAIsF3bHVy4hWn/GdFE+LCPkgb\nVIO0Qekz1nANYjl4kfPi/ImCB6VtHkp7aOAC/2QvFu8rs4F/+Mtyxsx+OjRH9LEM\nplyiA3hNmyccMXlSLnVJA7tQ1ub0fkGb9y9xRwKM7SP8CA+xvnsVjoq3qFp/Wyft\ngDyB/gLZE/TlXH5NftHi7qkSSb3ZHslFJjJbuG6q/Byv8/VgVk7rwpHRkGH6lPRm\nTZvAqI0nSR3+4eq586598kJcylrxFgcRysZRQUVVInnmKZcQqZIl24l4D5o0Q5al\na+RN1ICirl9Gc7TVjxntbt3JpSjFbFkN+LQNz3fiim+N6+jOC/CURQf9T45AWyWM\n+8zZCiMm20EA+WUGqbiSV4LAPT+3QMOGJZHhbwauipyww2s7Cg+2WJQOnJRzqQ6/\no0DQqX8v4otuhAPrDFvdex9tIJiFjicOrBTsuMe8DSPHMqYh8EktuI4WSKVHXs95\nujyVq1BFWfFacE4PyVXjbWXMBHC9Ln/HnwIbMmHNxZQF1vgPZitWITYGuODrZgaV\naNiNmnrgwlwxETidQlcgz7vbVdm6AQJjg8b8mGzI8Ms1w9JPx3HWjmHUW+bKgbHM\nTiOEKdwK1R+0yYUpcdaUb8g5tgH9QtBD+zz3amI9bncYjUYBzD9x9lJ5qEFg8F7z\nHhV8PolH4R9IoDZdJ5NW7hzfw2itcfGiNAn6FOwqjDRvfZz7T4GprFU4r6A5PIrW\nvXxayS0owhGxa46wjg0h+Pwz2ykuMJwXc6f7LQ2YfO83DAT/BdpA7NEKBA/u3cKG\ngANfOLN5Qx2EPnPvQ7QzQkhk+ICOVRdga5n8JL4FXK9anDSCdBNjrw==\n-----END RSA PRIVATE KEY-----\n'; }
function rsaPrivateAes256Pem() { return '-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\nDEK-Info: AES-256-CBC,520C22EC6F81EC4DA4E87A99C3B0EDBB\n\nkXE+R6HCuJ+Pl1O79NeU0Zpx/NGzD51NxmHAN6R2ky7zyX3DHhzXvnrVEfXdTIAm\np7Ptg+BMiZyynLPYP3C5okveUm0RL+K0SLdcssczxYMw8Xrvo1wNVsX4M0ISy+h5\nvPPLlDlot7BQwWWlnXC4rqJIo0N6Z2KYtDXiDCXTQA870PHGEGHTTey7Cpi82tOS\nrBVZElzIJ0rVc1i+CSjAFlyctvD75fy+ixZ45Npl50UUMOfHCu8zc3jetLU+1jxy\nbBkGvhKhhCuUhaNRjhhITiqJcvfNzvuVmjaiIFVC4LWX9p5aq8c2aDPtt10gqRKZ\n6sH5t3DUYSwvsPQUPrIDpeGapfs3wifeqmdyTkOynk43fvurENUM2MLX4WTs5VG1\nAHZHAMWR2HeX1nECDGaAr2Qkw5B9y1gUbIugDoq9rD27EDwqbfiCmJWJfWTsePtJ\nGNvckfDEkAkmlPg1j0oWpMM4G0ojFV/oiyKePebG82rZDuUINM5UFNd+z3jXe+0J\nw67ZKJvOtneITWL3MKsyjJbc9V6LlsH3xJcGKsSrfFhXF4iPPaHjsGC5q6Snx6IX\nd/S9RUcmddcmNnZHW48v8MI8wWc+ysEyodPM+zPOk+pyBVPkUcMmpv1HVaNdv3sZ\nhTij2VxXfuBKU9wp83PtQf3vZcMFkD2J/JHuuAYXrvhxOAHrIl6TWsJBuZa6CChZ\nB9myNPbPb8b14VuYNjdjec8USsnZPt3U6QP38cEO2WUZeLVBsk9zUZK5Dkj5phy5\nPBqlIBL0u+EmcUubYYD86aqTomdH5uQkM2wMCdNmM7Xb1g5j06lpyRfdSISZG/ps\nNu4KxxJGCWa6i5NswtbkyuXExXOjt8CeCLvBjotysUYPnL0AC56bWQG1LNanV3Ef\nIUip05vJTFSz9jIoB5tCrjSyb4vZ9WtFhOF5I5w9sPmhvinD+AjT//FLF4+O6Wue\nWQxt0Gyj5BSOjh7nOrQZGVXHNhrQJkAQZMv07OXMoJjgeA7I87Y6WWukv+9QRXn2\nviBN5I7Iq63VQXzN2RVUHKbZPIgu9P0IwyMHrnfUwnDN8074NeDpRPw6snSAsarZ\nPzh0ZuvxXV4iS17P4VnGJ1oCZnkKPU4phvH00ZkQg0mXIy4tH9tn/MhQpy9GQEmh\n//PtfKKAp7s9ctVqDNwNjt1GY++gHP9GmLOdFS2AJFipQqj6mzM+foOsNyhalIqS\ntXXLaSUAt89F4JED6A8xDJMFANUm5NKmNOTFcbeNzeEjq8gxPZXtCMxyH8k77vwR\n9oLKZMtToD8yF09sCAM9vQyztwEgV/u7n+9x1t51YnODmVrIsvQbYBU/HxmMUQd7\nqNkfCugPprJ7OhMNbWafnTeyPacr30AsEcJ7IgOxGPY3rt2VPxObI14clPbVEz+1\nO4NMzbaSYHwILAkxyNIrnf0ZzXOXKkgN0elMxnc2AwDkjm79R0KGAh4+yLxWPlxo\nfPgzxmo+3lI7fpILxwotzXnPXV2YxI7QIeFA+/q0kC8NTw2d0WoSDm4m8N0J7MR6\npbO6Mu0/vl5u6p8WhJKnxPOrSWVtTsDo2vm99fSkBHyUnbqqJr/FwUvaTWBAh/Lk\n-----END RSA PRIVATE KEY-----\n'; }
function legacyRoundTrip(pem) {
const priv = __thaw_crypto_module.createPrivateKey({ key: pem, passphrase: 'hunter2' });
const sig = __thaw_crypto_module.createSign('sha256').update('legacy pem test').sign(priv, 'hex');
const ok = __thaw_crypto_module.createVerify('sha256').update('legacy pem test').verify(rsaPublicPem(), sig, 'hex');
let wrongPassphraseThrew = false;
try { __thaw_crypto_module.createPrivateKey({ key: pem, passphrase: 'wrong' }); } catch (error) { wrongPassphraseThrew = true; }
return [priv.asymmetricKeyType, ok, wrongPassphraseThrew];
}
function des3RoundTrip() { return legacyRoundTrip(rsaPrivateDes3Pem()); }
function aes256RoundTrip() { return legacyRoundTrip(rsaPrivateAes256Pem()); }"##,
        r##"
var m = __thaw_crypto_module;
var des = m.createPrivateKey({ key: rsaPrivateDes3Pem(), passphrase: 'hunter2' });
var aes = m.createPrivateKey({ key: rsaPrivateAes256Pem(), passphrase: 'hunter2' });
var desWrong = false, aesWrong = false;
try { m.createPrivateKey({ key: rsaPrivateDes3Pem(), passphrase: 'wrong' }); } catch (error) { desWrong = true; }
try { m.createPrivateKey({ key: rsaPrivateAes256Pem(), passphrase: 'wrong' }); } catch (error) { aesWrong = true; }
console.log(JSON.stringify([des.asymmetricKeyType, aes.asymmetricKeyType, desWrong, aesWrong]));
"##,
    ]
    .concat();
    assert_eq!(run(&program), r#"["rsa","rsa",true,true]"#);
}

#[test]
fn asym_x25519_cannot_sign_and_secret_key_export() {
    let program = [
        r##"function x25519PrivateEncryptedPem() { return '-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIGjMF8GCSqGSIb3DQEFDTBSMDEGCSqGSIb3DQEFDDAkBBB7tmOZrnczjFnNG9jN\nYZ1fAgIIADAMBggqhkiG9w0CCQUAMB0GCWCGSAFlAwQBKgQQ7nt4x5kr1Oyp/30P\nhjd1KwRAPOtfFpXXehOtGmKdUDrNZt0kN1BGoevUg1UaV6IifpLoD/7rx0yKDYi7\nj3Mlx1DVoubvAvNgDari+t5VGInS5w==\n-----END ENCRYPTED PRIVATE KEY-----\n'; }
function peerPublicPem() { return '-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VuAyEA6PZLnkCbBlDxzjBJOcUA+J4MLON3SqHkcn7rLRsauks=\n-----END PUBLIC KEY-----\n'; }
function x25519PassphraseRoundTrip() {
let wrongPassphraseThrew = false;
try { __thaw_crypto_module.createPrivateKey({ key: x25519PrivateEncryptedPem(), passphrase: 'wrong' }); } catch (error) { wrongPassphraseThrew = true; }
const priv = __thaw_crypto_module.createPrivateKey({ key: x25519PrivateEncryptedPem(), passphrase: 'hunter2' });
const secret = __thaw_crypto_module.diffieHellman({
privateKey: priv,
publicKey: __thaw_crypto_module.createPublicKey(peerPublicPem())
}).toString('hex');
return [priv.asymmetricKeyType, secret, wrongPassphraseThrew];
}"##,
        r##"
var m = __thaw_crypto_module;
var CANNOT = "an X25519 key cannot sign -- it's a Diffie-Hellman key-agreement key, not a signing key";
var VERIFY_CANNOT = "an X25519 key cannot verify -- it's a Diffie-Hellman key-agreement key, not a signing key";
var peer = m.createPublicKey(peerPublicPem());
var signErr = '', verifyErr = '';
try { m.sign(null, 'hi', peer); } catch (error) { signErr = error.message; }
try { m.verify(null, 'hi', peer, Buffer.alloc(64)); } catch (error) { verifyErr = error.message; }
var x = m.createPrivateKey({ key: x25519PrivateEncryptedPem(), passphrase: 'hunter2' });
var encErr = '';
try { m.createSign('sha256').update('hi').sign(x); } catch (error) { encErr = error.message; }
var secret = m.createSecretKey(Buffer.from('secret-key'));
var viaKeyObject = m.createHmac('sha256', secret).update('payload').digest('hex');
var viaBuffer = m.createHmac('sha256', Buffer.from('secret-key')).update('payload').digest('hex');
console.log(JSON.stringify([signErr === CANNOT, verifyErr === VERIFY_CANNOT, x.asymmetricKeyType, encErr === CANNOT,
  secret.type, secret.export().toString(), secret.asymmetricKeyType === undefined, viaKeyObject === viaBuffer]));
"##,
    ]
    .concat();
    assert_eq!(
        run(&program),
        r#"[true,true,"x25519",true,"secret","secret-key",true,true]"#
    );
}
