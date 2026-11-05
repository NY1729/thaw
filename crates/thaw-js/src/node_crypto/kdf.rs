//! Password-based KDFs for `node:crypto`: PBKDF2 (RFC 8018) over the `digest.rs` HMAC, and
//! scrypt (RFC 7914) through the `scrypt` crate. Copied from thaw-quickjs (lib.rs `pbkdf2_bytes`,
//! quickjs/context.rs `scrypt_hex`), which stays the oracle until QuickJS is removed.

use super::digest::{digest_bytes, hmac_bytes};

/// PBKDF2 with `algorithm` (canonical name); `None` = unsupported digest.
/// ponytail: sha1/sha256/sha384/sha512 use the `pbkdf2` crate (HMAC key state reused across
/// iterations): the in-house loop below takes ~9.5 s in a debug build for 600000 iterations.
/// md5/sha3/ripemd keep the in-house loop.
pub(crate) fn pbkdf2_bytes(
    algorithm: &str,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    length: usize,
) -> Option<Vec<u8>> {
    if iterations == 0 {
        return Some(Vec::new());
    }
    let mut output = vec![0; length];
    match algorithm {
        "sha1" => pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, salt, iterations, &mut output),
        "sha256" => pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password, salt, iterations, &mut output),
        "sha384" => pbkdf2::pbkdf2_hmac::<sha2::Sha384>(password, salt, iterations, &mut output),
        "sha512" => pbkdf2::pbkdf2_hmac::<sha2::Sha512>(password, salt, iterations, &mut output),
        _ => return pbkdf2_in_house(algorithm, password, salt, iterations, length),
    }
    Some(output)
}

/// The QuickJS PBKDF2 loop (RFC 8018 section 5.2) over `hmac_bytes`; `None` = unsupported digest.
pub(crate) fn pbkdf2_in_house(
    algorithm: &str,
    password: &[u8],
    salt: &[u8],
    iterations: u32,
    length: usize,
) -> Option<Vec<u8>> {
    let digest_length = digest_bytes(algorithm, &[])?.len();
    if iterations == 0 {
        return Some(Vec::new());
    }
    let mut output = Vec::with_capacity(length);
    for block in 1..=length.div_ceil(digest_length) {
        let mut input = salt.to_vec();
        input.extend_from_slice(&(block as u32).to_be_bytes());
        let mut value = hmac_bytes(algorithm, password, &input)?;
        let mut accumulated = value.clone();
        for _ in 1..iterations {
            value = hmac_bytes(algorithm, password, &value)?;
            for (byte, next) in accumulated.iter_mut().zip(&value) {
                *byte ^= next;
            }
        }
        output.extend_from_slice(&accumulated);
    }
    output.truncate(length);
    Some(output)
}

/// Same message shape as the QuickJS native's `rquickjs` conversion error (thrown as TypeError).
fn conversion_message(from: &str, to: &str, message: &str) -> String {
    format!("Error converting from js '{from}' into type '{to}': {message}")
}

/// scrypt with the QuickJS wrapper's parameter checks and messages.
pub(crate) fn scrypt_bytes(
    password: &[u8],
    salt: &[u8],
    length: u32,
    cost: u32,
    block_size: u32,
    parallelization: u32,
) -> Result<Vec<u8>, String> {
    if cost < 2 || !cost.is_power_of_two() {
        return Err(conversion_message(
            "scrypt cost",
            "power of two greater than one",
            &cost.to_string(),
        ));
    }
    let params = scrypt::Params::new(
        cost.ilog2() as u8,
        block_size,
        parallelization,
        (length as usize).clamp(10, 64),
    )
    .map_err(|error| {
        conversion_message(
            "scrypt options",
            "valid scrypt options",
            &format!("{error}: N={cost}, r={block_size}, p={parallelization}, length={length}"),
        )
    })?;
    let mut output = vec![0; length as usize];
    scrypt::scrypt(password, salt, &params, &mut output)
        .map_err(|error| conversion_message("scrypt input", "derived key", &error.to_string()))?;
    Ok(output)
}
