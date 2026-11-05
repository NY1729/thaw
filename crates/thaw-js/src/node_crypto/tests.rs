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
