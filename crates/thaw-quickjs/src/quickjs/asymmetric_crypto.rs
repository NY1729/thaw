// RSA/ECDSA/EdDSA support for `node:crypto`'s `createSign`/
// `createVerify`/`crypto.sign`/`crypto.verify`
// (`platform_globals/buffer_crypto.js`). Matches this crate's existing
// "honest partial support" precedent: sign/verify (RSA PKCS1v15+PSS,
// ECDSA P-256/P-384/P-521 DER-encoded matching real Node's own default
// `dsaEncoding: 'der'`, Ed25519/EdDSA), PEM and DER key input,
// passphrase-protected PKCS8 private keys. No encrypt/decrypt or key
// generation here (see `crypto_asymmetric_encrypt_hex`/
// `crypto_generate_key_pair_json` for those, added alongside this).
// Still out of scope: P-521's `SecretKey` is only reachable via the
// generic `elliptic_curve`/`ecdsa` crates (its own `ecdsa::SigningKey`/
// `VerifyingKey` newtypes don't implement `pkcs8`'s decode traits
// directly, unlike P-256/P-384 -- see `parse_ec_p521_private`/
// `parse_ec_p521_public` below), Ed448, the legacy OpenSSL
// "Proc-Type: 4,ENCRYPTED" PKCS#1/SEC1 passphrase format (only modern
// PKCS#8 `ENCRYPTED PRIVATE KEY` is supported).
//
// Stateless and bytes-in/bytes-out, matching every other native
// crypto primitive in this crate (`digest_bytes`/`hmac_bytes` in
// `lib.rs`): no persistent native key objects are kept across calls.
// Import (`crypto_import_key_json`) is the one place PEM/DER/
// passphrase are actually resolved -- it re-encodes whatever it parses
// as a plain, decrypted PKCS8/SPKI PEM string, which is what gets
// carried on the JS-side `KeyObject` and handed back to every other
// function here unchanged, so `crypto_asymmetric_sign_hex`/`_verify`/
// `_pss`/encrypt/decrypt never need to know about DER or passphrases
// at all.

use pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey};
use pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::{RsaPrivateKey, RsaPublicKey};
use sec1::DecodeEcPrivateKey;
use signature::{SignatureEncoding, Signer, Verifier};

enum ParsedKey {
    RsaPrivate(Box<RsaPrivateKey>),
    RsaPublic(Box<RsaPublicKey>),
    EcPrivateP256(Box<p256::ecdsa::SigningKey>),
    EcPublicP256(Box<p256::ecdsa::VerifyingKey>),
    EcPrivateP384(Box<p384::ecdsa::SigningKey>),
    EcPublicP384(Box<p384::ecdsa::VerifyingKey>),
    EcPrivateP521(Box<p521::ecdsa::SigningKey>),
    EcPublicP521(Box<p521::ecdsa::VerifyingKey>),
    Ed25519Private(Box<ed25519_dalek::SigningKey>),
    Ed25519Public(Box<ed25519_dalek::VerifyingKey>),
}

impl ParsedKey {
    fn key_type(&self) -> &'static str {
        match self {
            ParsedKey::RsaPrivate(_) | ParsedKey::RsaPublic(_) => "rsa",
            ParsedKey::EcPrivateP256(_)
            | ParsedKey::EcPublicP256(_)
            | ParsedKey::EcPrivateP384(_)
            | ParsedKey::EcPublicP384(_)
            | ParsedKey::EcPrivateP521(_)
            | ParsedKey::EcPublicP521(_) => "ec",
            ParsedKey::Ed25519Private(_) | ParsedKey::Ed25519Public(_) => "ed25519",
        }
    }

    fn is_private(&self) -> bool {
        matches!(
            self,
            ParsedKey::RsaPrivate(_)
                | ParsedKey::EcPrivateP256(_)
                | ParsedKey::EcPrivateP384(_)
                | ParsedKey::EcPrivateP521(_)
                | ParsedKey::Ed25519Private(_)
        )
    }

    fn curve(&self) -> Option<&'static str> {
        match self {
            ParsedKey::EcPrivateP256(_) | ParsedKey::EcPublicP256(_) => Some("P-256"),
            ParsedKey::EcPrivateP384(_) | ParsedKey::EcPublicP384(_) => Some("P-384"),
            ParsedKey::EcPrivateP521(_) | ParsedKey::EcPublicP521(_) => Some("P-521"),
            ParsedKey::RsaPrivate(_)
            | ParsedKey::RsaPublic(_)
            | ParsedKey::Ed25519Private(_)
            | ParsedKey::Ed25519Public(_) => None,
        }
    }
}

/// P-521's own `ecdsa::SigningKey`/`VerifyingKey` newtypes don't
/// implement `pkcs8`'s decode traits directly (unlike P-256/P-384) --
/// go through the generic `elliptic_curve::SecretKey<NistP521>`/
/// `PublicKey<NistP521>` (which do) and convert via the generic
/// `ecdsa` crate's own `SigningKey<C>: From<SecretKey<C>>`/
/// `VerifyingKey<C>: From<PublicKey<C>>`, then P-521's own
/// `SigningKey: From<ecdsa::SigningKey<NistP521>>` newtype wrapper.
fn parse_ec_p521_private_pem(pem: &str) -> Option<p521::ecdsa::SigningKey> {
    let secret = p521::SecretKey::from_pkcs8_pem(pem)
        .or_else(|_| p521::SecretKey::from_sec1_pem(pem))
        .ok()?;
    let core: ecdsa::SigningKey<p521::NistP521> = secret.into();
    Some(core.into())
}

fn parse_ec_p521_public_pem(pem: &str) -> Option<p521::ecdsa::VerifyingKey> {
    let public = p521::PublicKey::from_public_key_pem(pem).ok()?;
    let core: ecdsa::VerifyingKey<p521::NistP521> = public.into();
    Some(core.into())
}

/// Tries every supported private-then-public key shape in turn --
/// there is no cheap way to know a PEM's real key type without fully
/// parsing it as each candidate, and this only ever runs on a short,
/// human-scale PEM block (never hot-path data).
fn parse_key(pem: &str) -> Option<ParsedKey> {
    if let Ok(key) = RsaPrivateKey::from_pkcs8_pem(pem) {
        return Some(ParsedKey::RsaPrivate(Box::new(key)));
    }
    if let Ok(key) = RsaPrivateKey::from_pkcs1_pem(pem) {
        return Some(ParsedKey::RsaPrivate(Box::new(key)));
    }
    if let Ok(key) = p256::ecdsa::SigningKey::from_pkcs8_pem(pem) {
        return Some(ParsedKey::EcPrivateP256(Box::new(key)));
    }
    if let Ok(key) = p256::ecdsa::SigningKey::from_sec1_pem(pem) {
        return Some(ParsedKey::EcPrivateP256(Box::new(key)));
    }
    if let Ok(key) = p384::ecdsa::SigningKey::from_pkcs8_pem(pem) {
        return Some(ParsedKey::EcPrivateP384(Box::new(key)));
    }
    if let Ok(key) = p384::ecdsa::SigningKey::from_sec1_pem(pem) {
        return Some(ParsedKey::EcPrivateP384(Box::new(key)));
    }
    if let Some(key) = parse_ec_p521_private_pem(pem) {
        return Some(ParsedKey::EcPrivateP521(Box::new(key)));
    }
    if let Ok(key) = ed25519_dalek::SigningKey::from_pkcs8_pem(pem) {
        return Some(ParsedKey::Ed25519Private(Box::new(key)));
    }
    if let Ok(key) = RsaPublicKey::from_public_key_pem(pem) {
        return Some(ParsedKey::RsaPublic(Box::new(key)));
    }
    if let Ok(key) = RsaPublicKey::from_pkcs1_pem(pem) {
        return Some(ParsedKey::RsaPublic(Box::new(key)));
    }
    if let Ok(key) = p256::ecdsa::VerifyingKey::from_public_key_pem(pem) {
        return Some(ParsedKey::EcPublicP256(Box::new(key)));
    }
    if let Ok(key) = p384::ecdsa::VerifyingKey::from_public_key_pem(pem) {
        return Some(ParsedKey::EcPublicP384(Box::new(key)));
    }
    if let Some(key) = parse_ec_p521_public_pem(pem) {
        return Some(ParsedKey::EcPublicP521(Box::new(key)));
    }
    if let Ok(key) = ed25519_dalek::VerifyingKey::from_public_key_pem(pem) {
        return Some(ParsedKey::Ed25519Public(Box::new(key)));
    }
    None
}

/// Backs `createPrivateKey`/`createPublicKey`'s validation and
/// `KeyObject.asymmetricKeyType`/`asymmetricKeyDetails`.
pub(crate) fn crypto_key_info_json(pem: &str) -> String {
    match parse_key(pem) {
        Some(key) => {
            let curve = key
                .curve()
                .map(|curve| format!(r#","namedCurve":"{curve}""#))
                .unwrap_or_default();
            format!(
                r#"{{"valid":true,"keyType":"{}","isPrivate":{}{curve}}}"#,
                key.key_type(),
                key.is_private()
            )
        }
        None => r#"{"valid":false}"#.to_string(),
    }
}

fn digest_size_matches(digest_algorithm: &str) -> Result<(), String> {
    if matches!(digest_algorithm, "sha1" | "sha256" | "sha384" | "sha512") {
        Ok(())
    } else {
        Err(format!("unsupported digest algorithm: {digest_algorithm}"))
    }
}

/// RSA-PKCS1v15 (default), ECDSA (P-256/P-384/P-521, DER-encoded), or
/// Ed25519/EdDSA sign, chosen by the parsed key's own type --
/// `digest_algorithm` only matters for RSA (ECDSA always uses its
/// curve's natural digest, the same pairing JOSE/JWT's ES256/ES384/
/// ES512 always use; EdDSA hashes internally with no external digest
/// choice at all, matching real Node's own `crypto.sign(null, data,
/// key)` for an Ed25519 key).
pub(crate) fn crypto_asymmetric_sign_hex(
    digest_algorithm: &str,
    pem: &str,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    match parse_key(pem).ok_or("invalid or unsupported private key")? {
        ParsedKey::RsaPrivate(key) => {
            digest_size_matches(digest_algorithm)?;
            sign_rsa_pkcs1v15(&key, digest_algorithm, data)
        }
        ParsedKey::EcPrivateP256(key) => {
            let signature: p256::ecdsa::Signature = key.sign(data);
            Ok(signature.to_der().to_vec())
        }
        ParsedKey::EcPrivateP384(key) => {
            let signature: p384::ecdsa::Signature = key.sign(data);
            Ok(signature.to_der().to_vec())
        }
        ParsedKey::EcPrivateP521(key) => {
            let signature: p521::ecdsa::Signature = key.sign(data);
            Ok(signature.to_der().to_vec())
        }
        ParsedKey::Ed25519Private(key) => {
            let signature: ed25519_dalek::Signature = key.sign(data);
            Ok(signature.to_vec())
        }
        ParsedKey::RsaPublic(_)
        | ParsedKey::EcPublicP256(_)
        | ParsedKey::EcPublicP384(_)
        | ParsedKey::EcPublicP521(_)
        | ParsedKey::Ed25519Public(_) => Err("a public key cannot sign".to_string()),
    }
}

pub(crate) fn crypto_asymmetric_sign_pss_hex(
    digest_algorithm: &str,
    pem: &str,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    digest_size_matches(digest_algorithm)?;
    let ParsedKey::RsaPrivate(key) = parse_key(pem).ok_or("invalid or unsupported private key")?
    else {
        return Err("RSA-PSS requires an RSA private key".to_string());
    };
    sign_rsa_pss(&key, digest_algorithm, data)
}

/// Mirrors `crypto_asymmetric_sign_hex`: verifies `signature` against
/// `data` using the key's own natural algorithm.
pub(crate) fn crypto_asymmetric_verify(
    digest_algorithm: &str,
    pem: &str,
    data: &[u8],
    signature: &[u8],
) -> Result<bool, String> {
    let key = parse_key(pem).ok_or("invalid or unsupported public/private key")?;
    Ok(match key {
        ParsedKey::RsaPrivate(key) => {
            digest_size_matches(digest_algorithm)?;
            verify_rsa_pkcs1v15(&key.to_public_key(), digest_algorithm, data, signature)
        }
        ParsedKey::RsaPublic(key) => {
            digest_size_matches(digest_algorithm)?;
            verify_rsa_pkcs1v15(&key, digest_algorithm, data, signature)
        }
        ParsedKey::EcPrivateP256(key) => p256::ecdsa::Signature::from_der(signature)
            .is_ok_and(|sig| key.verifying_key().verify(data, &sig).is_ok()),
        ParsedKey::EcPublicP256(key) => p256::ecdsa::Signature::from_der(signature)
            .is_ok_and(|sig| key.verify(data, &sig).is_ok()),
        ParsedKey::EcPrivateP384(key) => p384::ecdsa::Signature::from_der(signature)
            .is_ok_and(|sig| key.verifying_key().verify(data, &sig).is_ok()),
        ParsedKey::EcPublicP384(key) => p384::ecdsa::Signature::from_der(signature)
            .is_ok_and(|sig| key.verify(data, &sig).is_ok()),
        ParsedKey::EcPrivateP521(key) => p521::ecdsa::Signature::from_der(signature)
            .is_ok_and(|sig| p521::ecdsa::VerifyingKey::from(&*key).verify(data, &sig).is_ok()),
        ParsedKey::EcPublicP521(key) => p521::ecdsa::Signature::from_der(signature)
            .is_ok_and(|sig| key.verify(data, &sig).is_ok()),
        ParsedKey::Ed25519Private(key) => ed25519_dalek::Signature::from_slice(signature)
            .is_ok_and(|sig| key.verifying_key().verify(data, &sig).is_ok()),
        ParsedKey::Ed25519Public(key) => ed25519_dalek::Signature::from_slice(signature)
            .is_ok_and(|sig| key.verify(data, &sig).is_ok()),
    })
}

pub(crate) fn crypto_asymmetric_verify_pss(
    digest_algorithm: &str,
    pem: &str,
    data: &[u8],
    signature: &[u8],
) -> Result<bool, String> {
    digest_size_matches(digest_algorithm)?;
    let key = match parse_key(pem).ok_or("invalid or unsupported public/private key")? {
        ParsedKey::RsaPrivate(key) => key.to_public_key(),
        ParsedKey::RsaPublic(key) => *key,
        _ => return Err("RSA-PSS requires an RSA key".to_string()),
    };
    verify_rsa_pss(&key, digest_algorithm, data, signature)
}

macro_rules! rsa_pkcs1v15_sign_for {
    ($key:expr, $digest:ty, $data:expr) => {{
        let signing_key = rsa::pkcs1v15::SigningKey::<$digest>::new($key.clone());
        let signature: rsa::pkcs1v15::Signature = signing_key.sign($data);
        Ok(signature.to_vec())
    }};
}

fn sign_rsa_pkcs1v15(
    key: &RsaPrivateKey,
    digest_algorithm: &str,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    match digest_algorithm {
        "sha1" => rsa_pkcs1v15_sign_for!(key, sha1::Sha1, data),
        "sha256" => rsa_pkcs1v15_sign_for!(key, sha2::Sha256, data),
        "sha384" => rsa_pkcs1v15_sign_for!(key, sha2::Sha384, data),
        "sha512" => rsa_pkcs1v15_sign_for!(key, sha2::Sha512, data),
        _ => unreachable!("checked by digest_size_matches"),
    }
}

macro_rules! rsa_pkcs1v15_verify_for {
    ($key:expr, $digest:ty, $data:expr, $signature:expr) => {{
        let Ok(signature) = rsa::pkcs1v15::Signature::try_from($signature) else {
            return false;
        };
        let verifying_key = rsa::pkcs1v15::VerifyingKey::<$digest>::new($key.clone());
        verifying_key.verify($data, &signature).is_ok()
    }};
}

fn verify_rsa_pkcs1v15(
    key: &RsaPublicKey,
    digest_algorithm: &str,
    data: &[u8],
    signature: &[u8],
) -> bool {
    match digest_algorithm {
        "sha1" => rsa_pkcs1v15_verify_for!(key, sha1::Sha1, data, signature),
        "sha256" => rsa_pkcs1v15_verify_for!(key, sha2::Sha256, data, signature),
        "sha384" => rsa_pkcs1v15_verify_for!(key, sha2::Sha384, data, signature),
        "sha512" => rsa_pkcs1v15_verify_for!(key, sha2::Sha512, data, signature),
        _ => unreachable!("checked by digest_size_matches"),
    }
}

macro_rules! rsa_pss_sign_for {
    ($key:expr, $digest:ty, $data:expr) => {{
        let signing_key = rsa::pss::SigningKey::<$digest>::new($key.clone());
        let signature: rsa::pss::Signature = signing_key.sign($data);
        Ok(signature.to_vec())
    }};
}

fn sign_rsa_pss(
    key: &RsaPrivateKey,
    digest_algorithm: &str,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    match digest_algorithm {
        "sha1" => rsa_pss_sign_for!(key, sha1::Sha1, data),
        "sha256" => rsa_pss_sign_for!(key, sha2::Sha256, data),
        "sha384" => rsa_pss_sign_for!(key, sha2::Sha384, data),
        "sha512" => rsa_pss_sign_for!(key, sha2::Sha512, data),
        _ => unreachable!("checked by digest_size_matches"),
    }
}

macro_rules! rsa_pss_verify_for {
    ($key:expr, $digest:ty, $data:expr, $signature:expr) => {{
        let Ok(signature) = rsa::pss::Signature::try_from($signature) else {
            return Ok(false);
        };
        let verifying_key = rsa::pss::VerifyingKey::<$digest>::new($key.clone());
        Ok(verifying_key.verify($data, &signature).is_ok())
    }};
}

fn verify_rsa_pss(
    key: &RsaPublicKey,
    digest_algorithm: &str,
    data: &[u8],
    signature: &[u8],
) -> Result<bool, String> {
    match digest_algorithm {
        "sha1" => rsa_pss_verify_for!(key, sha1::Sha1, data, signature),
        "sha256" => rsa_pss_verify_for!(key, sha2::Sha256, data, signature),
        "sha384" => rsa_pss_verify_for!(key, sha2::Sha384, data, signature),
        "sha512" => rsa_pss_verify_for!(key, sha2::Sha512, data, signature),
        _ => unreachable!("checked by digest_size_matches"),
    }
}
