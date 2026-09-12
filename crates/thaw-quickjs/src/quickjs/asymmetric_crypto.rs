// RSA/ECDSA support for `node:crypto`'s `createSign`/`createVerify`/
// `crypto.sign`/`crypto.verify` (`platform_globals/buffer_crypto.js`).
// A practical subset, matching this crate's existing "honest partial
// support" precedent (`buffer_crypto.js`'s own doc comments already
// documented the *lack* of this before now): sign/verify only, RSA
// (PKCS1v15 and PSS padding) and ECDSA (P-256/P-384, DER-encoded
// signatures matching real Node's own default `dsaEncoding: 'der'`).
// No encrypt/decrypt, no key generation, no DER (only PEM) key input,
// no passphrase-protected keys, no P-521/Ed25519/Ed448 -- each would be
// its own separately-scoped effort if a real package ever needs it.
//
// Stateless and PEM-text/bytes-in, bytes-out, matching every other
// native crypto primitive in this crate (`digest_bytes`/`hmac_bytes` in
// `lib.rs`): no persistent native key objects are kept across calls --
// the PEM text itself is carried on the JS-side `KeyObject` and
// reparsed on every `sign`/`verify` call. Simpler than caching a
// parsed key, and signing/verifying is not hot-path/high-frequency
// enough in a Lambda handler for the reparse cost to matter.

use p256::ecdsa::{SigningKey as P256SigningKey, VerifyingKey as P256VerifyingKey};
use p384::ecdsa::{SigningKey as P384SigningKey, VerifyingKey as P384VerifyingKey};
use pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey};
use pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::{RsaPrivateKey, RsaPublicKey};
use sec1::DecodeEcPrivateKey;
use signature::{SignatureEncoding, Signer, Verifier};

enum ParsedKey {
    RsaPrivate(Box<RsaPrivateKey>),
    RsaPublic(Box<RsaPublicKey>),
    EcPrivateP256(Box<P256SigningKey>),
    EcPublicP256(Box<P256VerifyingKey>),
    EcPrivateP384(Box<P384SigningKey>),
    EcPublicP384(Box<P384VerifyingKey>),
}

impl ParsedKey {
    fn key_type(&self) -> &'static str {
        match self {
            ParsedKey::RsaPrivate(_) | ParsedKey::RsaPublic(_) => "rsa",
            ParsedKey::EcPrivateP256(_)
            | ParsedKey::EcPublicP256(_)
            | ParsedKey::EcPrivateP384(_)
            | ParsedKey::EcPublicP384(_) => "ec",
        }
    }

    fn is_private(&self) -> bool {
        matches!(
            self,
            ParsedKey::RsaPrivate(_) | ParsedKey::EcPrivateP256(_) | ParsedKey::EcPrivateP384(_)
        )
    }

    fn curve(&self) -> Option<&'static str> {
        match self {
            ParsedKey::EcPrivateP256(_) | ParsedKey::EcPublicP256(_) => Some("P-256"),
            ParsedKey::EcPrivateP384(_) | ParsedKey::EcPublicP384(_) => Some("P-384"),
            ParsedKey::RsaPrivate(_) | ParsedKey::RsaPublic(_) => None,
        }
    }
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
    if let Ok(key) = P256SigningKey::from_pkcs8_pem(pem) {
        return Some(ParsedKey::EcPrivateP256(Box::new(key)));
    }
    if let Ok(key) = P256SigningKey::from_sec1_pem(pem) {
        return Some(ParsedKey::EcPrivateP256(Box::new(key)));
    }
    if let Ok(key) = P384SigningKey::from_pkcs8_pem(pem) {
        return Some(ParsedKey::EcPrivateP384(Box::new(key)));
    }
    if let Ok(key) = P384SigningKey::from_sec1_pem(pem) {
        return Some(ParsedKey::EcPrivateP384(Box::new(key)));
    }
    if let Ok(key) = RsaPublicKey::from_public_key_pem(pem) {
        return Some(ParsedKey::RsaPublic(Box::new(key)));
    }
    if let Ok(key) = RsaPublicKey::from_pkcs1_pem(pem) {
        return Some(ParsedKey::RsaPublic(Box::new(key)));
    }
    if let Ok(key) = P256VerifyingKey::from_public_key_pem(pem) {
        return Some(ParsedKey::EcPublicP256(Box::new(key)));
    }
    if let Ok(key) = P384VerifyingKey::from_public_key_pem(pem) {
        return Some(ParsedKey::EcPublicP384(Box::new(key)));
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

/// RSA-PKCS1v15 (default) or ECDSA (P-256/P-384, DER-encoded) sign,
/// chosen by the parsed key's own type -- `digest_algorithm` only
/// matters for RSA (ECDSA always uses its curve's natural digest, the
/// same pairing JOSE/JWT's ES256/ES384 always use).
pub(crate) fn crypto_asymmetric_sign_hex(
    digest_algorithm: &str,
    pem: &str,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    digest_size_matches(digest_algorithm)?;
    match parse_key(pem).ok_or("invalid or unsupported private key")? {
        ParsedKey::RsaPrivate(key) => sign_rsa_pkcs1v15(&key, digest_algorithm, data),
        ParsedKey::EcPrivateP256(key) => {
            let signature: p256::ecdsa::Signature = key.sign(data);
            Ok(signature.to_der().to_vec())
        }
        ParsedKey::EcPrivateP384(key) => {
            let signature: p384::ecdsa::Signature = key.sign(data);
            Ok(signature.to_der().to_vec())
        }
        ParsedKey::RsaPublic(_) | ParsedKey::EcPublicP256(_) | ParsedKey::EcPublicP384(_) => {
            Err("a public key cannot sign".to_string())
        }
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
    digest_size_matches(digest_algorithm)?;
    let key = parse_key(pem).ok_or("invalid or unsupported public/private key")?;
    Ok(match key {
        ParsedKey::RsaPrivate(key) => {
            verify_rsa_pkcs1v15(&key.to_public_key(), digest_algorithm, data, signature)
        }
        ParsedKey::RsaPublic(key) => verify_rsa_pkcs1v15(&key, digest_algorithm, data, signature),
        ParsedKey::EcPrivateP256(key) => p256::ecdsa::Signature::from_der(signature)
            .is_ok_and(|sig| key.verifying_key().verify(data, &sig).is_ok()),
        ParsedKey::EcPublicP256(key) => p256::ecdsa::Signature::from_der(signature)
            .is_ok_and(|sig| key.verify(data, &sig).is_ok()),
        ParsedKey::EcPrivateP384(key) => p384::ecdsa::Signature::from_der(signature)
            .is_ok_and(|sig| key.verifying_key().verify(data, &sig).is_ok()),
        ParsedKey::EcPublicP384(key) => p384::ecdsa::Signature::from_der(signature)
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
