//! `node:crypto` natives for thaw-js: digests, HMAC and OS randomness. The JS half is
//! `web/node_crypto.js`; the digest code is copied from thaw-quickjs (see `digest.rs`).

mod digest;
#[cfg(test)]
mod tests;

use crate::net::{n, s};
use crate::zlib::{hex_decode, hex_encode};
use crate::*;
use digest::{digest_bytes, hmac_bytes};

const SOURCE: &str = include_str!("../web/node_crypto.js");

/// Canonical name from a user-facing one: `SHA-256` -> `sha256`.
fn canonical(name: &str) -> String {
    name.to_ascii_lowercase().replace('-', "")
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
    interp.global.declare("__thaw_crypto_hash_hex", hash, true);
    interp.global.declare("__thaw_crypto_hmac_hex", hmac, true);
    interp
        .global
        .declare("__thaw_crypto_random_hex", random, true);
    interp
        .run(&format!("(function () {{\n{SOURCE}\n}})();"))
        .unwrap_or_else(|_| panic!("node:crypto prelude must evaluate"));
}
