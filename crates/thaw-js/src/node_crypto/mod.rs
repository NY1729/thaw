//! `node:crypto` natives for thaw-js: digests, HMAC and OS randomness. The JS half is
//! `web/node_crypto.js`; the digest code is copied from thaw-quickjs (see `digest.rs`).

mod asym;
mod cipher;
mod digest;
mod kdf;
#[cfg(test)]
mod tests;

use crate::net::{n, s};
use crate::zlib::{hex_decode, hex_encode};
use crate::*;
use asym::{
    crypto_asymmetric_decrypt_hex, crypto_asymmetric_encrypt_hex,
    crypto_asymmetric_private_encrypt_hex, crypto_asymmetric_public_decrypt_hex,
    crypto_asymmetric_sign_hex, crypto_asymmetric_sign_pss_hex, crypto_asymmetric_verify,
    crypto_asymmetric_verify_pss, crypto_diffie_hellman_hex, crypto_ecdh_compute_secret_hex,
    crypto_ecdh_generate_keys_hex, crypto_ecdh_public_from_private_hex,
    crypto_generate_key_pair_json, crypto_import_key_json,
};
use cipher::{sym_block_cipher, sym_ctr_xor, sym_gcm_tag, sym_gcm_xor};
use digest::{digest_bytes, hmac_bytes};
use kdf::{pbkdf2_bytes, scrypt_bytes};

const SOURCE: &str = include_str!("../web/node_crypto.js");

/// Canonical name from a user-facing one: `SHA-256` -> `sha256`.
fn canonical(name: &str) -> String {
    name.to_ascii_lowercase().replace('-', "")
}

/// A boolean argument (the natives take real JS booleans for their flags).
fn flag(a: &[Value], i: usize) -> bool {
    matches!(arg(a, i), Value::Bool(true))
}

/// Cipher errors cross as a `!reason` string, which the JS side turns into OpenSSL-style errors.
fn cipher_result(result: Result<Vec<u8>, String>) -> Value {
    Value::str(&match result {
        Ok(bytes) => hex_encode(&bytes),
        Err(reason) => format!("!{reason}"),
    })
}

pub(crate) fn install(interp: &mut Interp) {
    // `__thaw_crypto_hash_hex(alg, hex)`: digest of hex bytes as hex (also used by net_tests).
    let hash = interp.native(|it, _, a| {
        let alg = canonical(&s(it, a, 0));
        match digest_bytes(&alg, &hex_decode(&s(it, a, 1))) {
            Some(sum) => Ok(Value::str(&hex_encode(&sum))),
            None => it.throw_error("Error", &format!("Digest method not supported: {alg}")),
        }
    });
    // `__thaw_crypto_hmac_hex(alg, keyhex, datahex)`
    let hmac = interp.native(|it, _, a| {
        let alg = canonical(&s(it, a, 0));
        let (key, data) = (hex_decode(&s(it, a, 1)), hex_decode(&s(it, a, 2)));
        match hmac_bytes(&alg, &key, &data) {
            Some(mac) => Ok(Value::str(&hex_encode(&mac))),
            None => it.throw_error("Error", &format!("Digest method not supported: {alg}")),
        }
    });
    // `__thaw_crypto_random_hex(size)`: OS random bytes as hex; JS validates the size.
    let random = interp.native(|it, _, a| {
        let mut bytes = vec![0u8; n(it, a, 0) as usize];
        crate::web_crypto::fill_os_random(&mut bytes);
        Ok(Value::str(&hex_encode(&bytes)))
    });
    // `__thaw_crypto_pbkdf2_hex(alg, passwordhex, salthex, iterations, keylen)`; JS checks the numbers.
    let pbkdf2 = interp.native(|it, _, a| {
        let alg = canonical(&s(it, a, 0));
        let (password, salt) = (hex_decode(&s(it, a, 1)), hex_decode(&s(it, a, 2)));
        match pbkdf2_bytes(&alg, &password, &salt, n(it, a, 3), n(it, a, 4) as usize) {
            Some(key) => Ok(Value::str(&hex_encode(&key))),
            None => it.throw_error("Error", &format!("Digest method not supported: {alg}")),
        }
    });
    // `__thaw_crypto_scrypt_hex(passwordhex, salthex, keylen, N, r, p)`; errors are TypeErrors.
    let scrypt_hex = interp.native(|it, _, a| {
        let (password, salt) = (hex_decode(&s(it, a, 0)), hex_decode(&s(it, a, 1)));
        let (length, cost, block_size, parallelization) =
            (n(it, a, 2), n(it, a, 3), n(it, a, 4), n(it, a, 5));
        match scrypt_bytes(&password, &salt, length, cost, block_size, parallelization) {
            Ok(key) => Ok(Value::str(&hex_encode(&key))),
            Err(message) => it.throw_error("TypeError", &message),
        }
    });
    // `__thaw_crypto_cipher_hex(algorithm, keyhex, ivhex, datahex, decrypt, finalize, padding)`: CBC/ECB.
    let cipher_hex = interp.native(|it, _, a| {
        let algorithm = s(it, a, 0).to_ascii_lowercase();
        let result = sym_block_cipher(
            algorithm.ends_with("-ecb"),
            &hex_decode(&s(it, a, 1)),
            &hex_decode(&s(it, a, 2)),
            &hex_decode(&s(it, a, 3)),
            flag(a, 4),
            flag(a, 5),
            flag(a, 6),
        );
        Ok(cipher_result(result))
    });
    // `__thaw_crypto_ctr_hex(gcm, keyhex, ivhex, offset, datahex)`: CTR, or the GCM payload when `gcm`.
    let ctr_hex = interp.native(|it, _, a| {
        let (key, iv, value) = (
            hex_decode(&s(it, a, 1)),
            hex_decode(&s(it, a, 2)),
            hex_decode(&s(it, a, 4)),
        );
        let offset = it.num(&arg(a, 3)) as usize;
        let result = if flag(a, 0) {
            sym_gcm_xor(&key, &iv, offset, &value)
        } else {
            sym_ctr_xor(&key, &iv, offset, &value, true)
        };
        Ok(cipher_result(result))
    });
    // `__thaw_crypto_gcm_tag_hex(keyhex, ivhex, aadhex, ciphertexthex)`: the full 16-byte tag.
    let gcm_tag_hex = interp.native(|it, _, a| {
        let result = sym_gcm_tag(
            &hex_decode(&s(it, a, 0)),
            &hex_decode(&s(it, a, 1)),
            &hex_decode(&s(it, a, 2)),
            &hex_decode(&s(it, a, 3)),
        );
        Ok(cipher_result(result))
    });
    // `__thaw_crypto_import_key_json(keyhex, isDer, passphrase)`: normalized PEM plus key info as JSON.
    let import_key = interp.native(|it, _, a| {
        let json = crypto_import_key_json(&s(it, a, 0), flag(a, 1), &s(it, a, 2));
        Ok(Value::str(&json))
    });
    // `__thaw_crypto_asymmetric_sign_hex(alg, pem, datahex)` / `_sign_pss_hex`: signature as hex.
    let sign = interp.native(|it, _, a| {
        let result =
            crypto_asymmetric_sign_hex(&s(it, a, 0), &s(it, a, 1), &hex_decode(&s(it, a, 2)));
        match result {
            Ok(signature) => Ok(Value::str(&hex_encode(&signature))),
            Err(message) => it.throw_error("Error", &message),
        }
    });
    let sign_pss = interp.native(|it, _, a| {
        let result =
            crypto_asymmetric_sign_pss_hex(&s(it, a, 0), &s(it, a, 1), &hex_decode(&s(it, a, 2)));
        match result {
            Ok(signature) => Ok(Value::str(&hex_encode(&signature))),
            Err(message) => it.throw_error("Error", &message),
        }
    });
    // `__thaw_crypto_asymmetric_verify(alg, pem, datahex, sighex)` / `_verify_pss`: boolean.
    let verify = interp.native(|it, _, a| {
        let result = crypto_asymmetric_verify(
            &s(it, a, 0),
            &s(it, a, 1),
            &hex_decode(&s(it, a, 2)),
            &hex_decode(&s(it, a, 3)),
        );
        match result {
            Ok(valid) => Ok(Value::Bool(valid)),
            Err(message) => it.throw_error("Error", &message),
        }
    });
    let verify_pss = interp.native(|it, _, a| {
        let result = crypto_asymmetric_verify_pss(
            &s(it, a, 0),
            &s(it, a, 1),
            &hex_decode(&s(it, a, 2)),
            &hex_decode(&s(it, a, 3)),
        );
        match result {
            Ok(valid) => Ok(Value::Bool(valid)),
            Err(message) => it.throw_error("Error", &message),
        }
    });
    // `__thaw_crypto_asymmetric_encrypt_hex(pem, oaepDigest, datahex)` / `_decrypt_hex`: RSA;
    // an empty digest means PKCS1v15, otherwise OAEP with that hash.
    let encrypt = interp.native(|it, _, a| {
        match crypto_asymmetric_encrypt_hex(&s(it, a, 0), &s(it, a, 1), &hex_decode(&s(it, a, 2))) {
            Ok(value) => Ok(Value::str(&hex_encode(&value))),
            Err(message) => it.throw_error("Error", &message),
        }
    });
    let decrypt = interp.native(|it, _, a| {
        match crypto_asymmetric_decrypt_hex(&s(it, a, 0), &s(it, a, 1), &hex_decode(&s(it, a, 2))) {
            Ok(value) => Ok(Value::str(&hex_encode(&value))),
            Err(message) => it.throw_error("Error", &message),
        }
    });
    // `__thaw_crypto_asymmetric_private_encrypt_hex(pem, datahex)` / `_public_decrypt_hex`.
    let private_encrypt = interp.native(|it, _, a| {
        match crypto_asymmetric_private_encrypt_hex(&s(it, a, 0), &hex_decode(&s(it, a, 1))) {
            Ok(value) => Ok(Value::str(&hex_encode(&value))),
            Err(message) => it.throw_error("Error", &message),
        }
    });
    let public_decrypt = interp.native(|it, _, a| {
        match crypto_asymmetric_public_decrypt_hex(&s(it, a, 0), &hex_decode(&s(it, a, 1))) {
            Ok(value) => Ok(Value::str(&hex_encode(&value))),
            Err(message) => it.throw_error("Error", &message),
        }
    });
    // `__thaw_crypto_generate_key_pair_json(type, modulusOrCurve, publicExponent, privType, pubType)`
    let generate = interp.native(|it, _, a| {
        let result = crypto_generate_key_pair_json(
            &s(it, a, 0),
            &s(it, a, 1),
            &s(it, a, 2),
            &s(it, a, 3),
            &s(it, a, 4),
        );
        match result {
            Ok(json) => Ok(Value::str(&json)),
            Err(message) => it.throw_error("Error", &message),
        }
    });
    // `__thaw_crypto_diffie_hellman_hex(privatePem, publicPem)`: shared secret as hex.
    let diffie_hellman = interp.native(|it, _, a| {
        match crypto_diffie_hellman_hex(&s(it, a, 0), &s(it, a, 1)) {
            Ok(secret) => Ok(Value::str(&hex_encode(&secret))),
            Err(message) => it.throw_error("Error", &message),
        }
    });
    // ECDH (classic raw-byte API): `_generate_keys_hex(curve)` returns JSON, the others hex.
    let ecdh_generate = interp.native(|it, _, a| match crypto_ecdh_generate_keys_hex(&s(it, a, 0)) {
        Ok(json) => Ok(Value::str(&json)),
        Err(message) => it.throw_error("Error", &message),
    });
    let ecdh_public = interp.native(|it, _, a| {
        match crypto_ecdh_public_from_private_hex(&s(it, a, 0), &s(it, a, 1)) {
            Ok(public) => Ok(Value::str(&public)),
            Err(message) => it.throw_error("Error", &message),
        }
    });
    let ecdh_secret = interp.native(|it, _, a| {
        match crypto_ecdh_compute_secret_hex(&s(it, a, 0), &s(it, a, 1), &s(it, a, 2)) {
            Ok(secret) => Ok(Value::str(&secret)),
            Err(message) => it.throw_error("Error", &message),
        }
    });
    interp
        .global
        .declare("__thaw_crypto_asymmetric_encrypt_hex", encrypt, true);
    interp
        .global
        .declare("__thaw_crypto_asymmetric_decrypt_hex", decrypt, true);
    interp
        .global
        .declare("__thaw_crypto_asymmetric_private_encrypt_hex", private_encrypt, true);
    interp
        .global
        .declare("__thaw_crypto_asymmetric_public_decrypt_hex", public_decrypt, true);
    interp
        .global
        .declare("__thaw_crypto_generate_key_pair_json", generate, true);
    interp
        .global
        .declare("__thaw_crypto_diffie_hellman_hex", diffie_hellman, true);
    interp
        .global
        .declare("__thaw_crypto_ecdh_generate_keys_hex", ecdh_generate, true);
    interp
        .global
        .declare("__thaw_crypto_ecdh_public_from_private_hex", ecdh_public, true);
    interp
        .global
        .declare("__thaw_crypto_ecdh_compute_secret_hex", ecdh_secret, true);
    interp
        .global
        .declare("__thaw_crypto_import_key_json", import_key, true);
    interp
        .global
        .declare("__thaw_crypto_asymmetric_sign_hex", sign, true);
    interp
        .global
        .declare("__thaw_crypto_asymmetric_sign_pss_hex", sign_pss, true);
    interp
        .global
        .declare("__thaw_crypto_asymmetric_verify", verify, true);
    interp
        .global
        .declare("__thaw_crypto_asymmetric_verify_pss", verify_pss, true);
    interp
        .global
        .declare("__thaw_crypto_cipher_hex", cipher_hex, true);
    interp
        .global
        .declare("__thaw_crypto_ctr_hex", ctr_hex, true);
    interp
        .global
        .declare("__thaw_crypto_gcm_tag_hex", gcm_tag_hex, true);
    interp
        .global
        .declare("__thaw_crypto_pbkdf2_hex", pbkdf2, true);
    interp
        .global
        .declare("__thaw_crypto_scrypt_hex", scrypt_hex, true);
    interp.global.declare("__thaw_crypto_hash_hex", hash, true);
    interp.global.declare("__thaw_crypto_hmac_hex", hmac, true);
    interp
        .global
        .declare("__thaw_crypto_random_hex", random, true);
    interp
        .run(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("node:crypto prelude must evaluate"));
}
