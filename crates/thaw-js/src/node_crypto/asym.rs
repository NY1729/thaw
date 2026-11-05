//! Asymmetric keys and signatures for `node:crypto`: PEM/DER/encrypted PKCS8/legacy PEM import
//! (`crypto_import_key_json`), RSA PKCS1v15/PSS, ECDSA P-256/P-384/P-521 and Ed25519/Ed448
//! sign/verify. Copied from thaw-quickjs (quickjs/asymmetric_crypto.rs, which stays the oracle
//! until QuickJS is removed). Keygen, RSA encrypt/decrypt and DH are not here yet (PR6).

use crate::zlib::hex_decode;
use base64::Engine;
use pkcs1::{DecodeRsaPrivateKey, DecodeRsaPublicKey};
use pkcs8::{DecodePrivateKey, DecodePublicKey};
use rsa::{RsaPrivateKey, RsaPublicKey};
use sec1::DecodeEcPrivateKey;
use sha2::Digest;
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
    X25519Private(Box<x25519_dalek::StaticSecret>),
    X25519Public(Box<x25519_dalek::PublicKey>),
    Ed448Private(Box<ed448_goldilocks::SigningKey>),
    Ed448Public(Box<ed448_goldilocks::VerifyingKey>),
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
            ParsedKey::X25519Private(_) | ParsedKey::X25519Public(_) => "x25519",
            ParsedKey::Ed448Private(_) | ParsedKey::Ed448Public(_) => "ed448",
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
                | ParsedKey::X25519Private(_)
                | ParsedKey::Ed448Private(_)
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
            | ParsedKey::Ed25519Public(_)
            | ParsedKey::X25519Private(_)
            | ParsedKey::X25519Public(_)
            | ParsedKey::Ed448Private(_)
            | ParsedKey::Ed448Public(_) => None,
        }
    }

    /// `(modulusLength, publicExponent)` for real Node's
    /// `KeyObject.asymmetricKeyDetails` -- `publicExponent` as a decimal
    /// string, since it's presented as a `BigInt` on the JS side (a
    /// custom `generateKeyPairSync({ publicExponent })` value can
    /// exceed `f64`'s exact-integer range).
    fn rsa_details(&self) -> Option<(usize, String)> {
        use rsa::traits::PublicKeyParts;
        match self {
            ParsedKey::RsaPrivate(key) => Some((key.n().bits(), key.e().to_string())),
            ParsedKey::RsaPublic(key) => Some((key.n().bits(), key.e().to_string())),
            _ => None,
        }
    }

    /// Re-encodes as a plain, unencrypted PKCS8 (private)/SPKI (public)
    /// PEM string -- the normalized form `crypto_import_key_json`
    /// stores on the JS-side `KeyObject`, regardless of what format the
    /// original input was in.
    fn to_pem(&self) -> Option<String> {
        use pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding};
        match self {
            ParsedKey::RsaPrivate(key) => key
                .to_pkcs8_pem(LineEnding::LF)
                .ok()
                .map(|pem| pem.to_string()),
            ParsedKey::RsaPublic(key) => key.to_public_key_pem(LineEnding::LF).ok(),
            ParsedKey::EcPrivateP256(key) => key
                .to_pkcs8_pem(LineEnding::LF)
                .ok()
                .map(|pem| pem.to_string()),
            ParsedKey::EcPublicP256(key) => key.to_public_key_pem(LineEnding::LF).ok(),
            ParsedKey::EcPrivateP384(key) => key
                .to_pkcs8_pem(LineEnding::LF)
                .ok()
                .map(|pem| pem.to_string()),
            ParsedKey::EcPublicP384(key) => key.to_public_key_pem(LineEnding::LF).ok(),
            ParsedKey::EcPrivateP521(key) => ec_p521_private_to_pem(key),
            ParsedKey::EcPublicP521(key) => ec_p521_public_to_pem(key),
            ParsedKey::Ed25519Private(key) => key
                .to_pkcs8_pem(LineEnding::LF)
                .ok()
                .map(|pem| pem.to_string()),
            ParsedKey::Ed25519Public(key) => key.to_public_key_pem(LineEnding::LF).ok(),
            ParsedKey::X25519Private(key) => x25519_private_to_pem(key),
            ParsedKey::X25519Public(key) => x25519_public_to_pem(key),
            ParsedKey::Ed448Private(key) => ed448_private_to_pem(key),
            ParsedKey::Ed448Public(key) => ed448_public_to_pem(key),
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

fn parse_ec_p521_private_der(bytes: &[u8]) -> Option<p521::ecdsa::SigningKey> {
    let secret = p521::SecretKey::from_pkcs8_der(bytes)
        .or_else(|_| p521::SecretKey::from_sec1_der(bytes))
        .ok()?;
    let core: ecdsa::SigningKey<p521::NistP521> = secret.into();
    Some(core.into())
}

fn parse_ec_p521_public_der(bytes: &[u8]) -> Option<p521::ecdsa::VerifyingKey> {
    let public = p521::PublicKey::from_public_key_der(bytes).ok()?;
    let core: ecdsa::VerifyingKey<p521::NistP521> = public.into();
    Some(core.into())
}

fn parse_ec_p521_private_encrypted_pem(
    pem: &str,
    passphrase: &str,
) -> Option<p521::ecdsa::SigningKey> {
    let secret = p521::SecretKey::from_pkcs8_encrypted_pem(pem, passphrase).ok()?;
    let core: ecdsa::SigningKey<p521::NistP521> = secret.into();
    Some(core.into())
}

fn parse_ec_p521_private_encrypted_der(
    bytes: &[u8],
    passphrase: &str,
) -> Option<p521::ecdsa::SigningKey> {
    let secret = p521::SecretKey::from_pkcs8_encrypted_der(bytes, passphrase).ok()?;
    let core: ecdsa::SigningKey<p521::NistP521> = secret.into();
    Some(core.into())
}

/// Re-encodes a P-521 signing/verifying key as a plain PKCS8/SPKI PEM
/// string -- P-521's newtypes don't implement `pkcs8`'s encode traits
/// either, so this round-trips through the raw scalar/point bytes into
/// the generic `SecretKey`/`PublicKey` (which do), mirroring the
/// decode-side detour above.
fn ec_p521_private_to_pem(key: &p521::ecdsa::SigningKey) -> Option<String> {
    let secret = p521::SecretKey::from_bytes(&key.to_bytes()).ok()?;
    use pkcs8::EncodePrivateKey;
    secret
        .to_pkcs8_pem(pkcs8::LineEnding::LF)
        .ok()
        .map(|pem| pem.to_string())
}

fn ec_p521_public_to_pem(key: &p521::ecdsa::VerifyingKey) -> Option<String> {
    let public = p521::PublicKey::from_sec1_bytes(key.to_encoded_point(false).as_bytes()).ok()?;
    use pkcs8::EncodePublicKey;
    public.to_public_key_pem(pkcs8::LineEnding::LF).ok()
}

/// X25519's OID (RFC 8410 -- `id-X25519`); `x25519-dalek` has no
/// `pkcs8` feature at all (unlike `ed25519-dalek`), so this crate's
/// PKCS8/SPKI wrapping is hand-built the same way `ed25519`'s own
/// `pkcs8::KeypairBytes`/`PublicKeyBytes` do internally for Ed25519 --
/// same nested-OCTET-STRING private-key shape, same flat-bytes
/// SPKI `BIT STRING` public-key shape, different OID.
const X25519_ALGORITHM_ID: pkcs8::AlgorithmIdentifierRef<'static> = pkcs8::AlgorithmIdentifierRef {
    oid: pkcs8::ObjectIdentifier::new_unwrap("1.3.101.110"),
    parameters: None,
};

fn x25519_private_to_pem(secret: &x25519_dalek::StaticSecret) -> Option<String> {
    let mut private_key = [0u8; 34];
    private_key[0] = 0x04;
    private_key[1] = 0x20;
    private_key[2..].copy_from_slice(&secret.to_bytes());
    let public_key = x25519_dalek::PublicKey::from(secret).to_bytes();
    let info = pkcs8::PrivateKeyInfo {
        algorithm: X25519_ALGORITHM_ID,
        private_key: &private_key,
        public_key: Some(&public_key),
    };
    let document = pkcs8::SecretDocument::encode_msg(&info).ok()?;
    document
        .to_pem("PRIVATE KEY", pkcs8::LineEnding::LF)
        .ok()
        .map(|pem| pem.to_string())
}

fn x25519_public_to_pem(public: &x25519_dalek::PublicKey) -> Option<String> {
    let bytes = public.to_bytes();
    let spki = pkcs8::SubjectPublicKeyInfoRef {
        algorithm: X25519_ALGORITHM_ID,
        subject_public_key: pkcs8::der::asn1::BitStringRef::new(0, &bytes).ok()?,
    };
    let document: pkcs8::Document = spki.try_into().ok()?;
    document.to_pem("PUBLIC KEY", pkcs8::LineEnding::LF).ok()
}

fn parse_x25519_private_pem(pem: &str) -> Option<x25519_dalek::StaticSecret> {
    let (label, document) = pkcs8::SecretDocument::from_pem(pem).ok()?;
    if label != "PRIVATE KEY" {
        return None;
    }
    parse_x25519_private_der(document.as_bytes())
}

fn parse_x25519_private_der(bytes: &[u8]) -> Option<x25519_dalek::StaticSecret> {
    use pkcs8::der::Decode;
    let info = pkcs8::PrivateKeyInfo::from_der(bytes).ok()?;
    if info.algorithm.oid != X25519_ALGORITHM_ID.oid {
        return None;
    }
    let scalar: [u8; 32] = match info.private_key {
        [0x04, 0x20, rest @ ..] => rest.try_into().ok()?,
        _ => return None,
    };
    Some(x25519_dalek::StaticSecret::from(scalar))
}

/// Passphrase-protected X25519 keys: `x25519-dalek` never had a
/// `pkcs8` feature to derive `from_pkcs8_encrypted_pem`/`_der` from in
/// the first place (this whole module's own X25519 PKCS8 wrapping is
/// hand-built, see `X25519_ALGORITHM_ID`'s doc comment above), but the
/// *decryption* step itself has nothing curve-specific about it --
/// `pkcs8::EncryptedPrivateKeyInfo::decrypt` (modern PBES2, the same
/// primitive Ed448's own `parse_ed448_private_encrypted_der` already
/// reuses for the identical reason) hands back a plain decrypted PKCS8
/// `SecretDocument` regardless of what algorithm is inside, ready for
/// `parse_x25519_private_der` exactly as if it had never been
/// encrypted at all.
fn parse_x25519_private_encrypted_pem(
    pem: &str,
    passphrase: &str,
) -> Option<x25519_dalek::StaticSecret> {
    let (label, encrypted) = pkcs8::Document::from_pem(pem).ok()?;
    if label != "ENCRYPTED PRIVATE KEY" {
        return None;
    }
    parse_x25519_private_encrypted_der(encrypted.as_bytes(), passphrase)
}

fn parse_x25519_private_encrypted_der(
    bytes: &[u8],
    passphrase: &str,
) -> Option<x25519_dalek::StaticSecret> {
    let document = pkcs8::EncryptedPrivateKeyInfo::try_from(bytes)
        .ok()?
        .decrypt(passphrase)
        .ok()?;
    parse_x25519_private_der(document.as_bytes())
}

fn parse_x25519_public_pem(pem: &str) -> Option<x25519_dalek::PublicKey> {
    let (label, document) = pkcs8::Document::from_pem(pem).ok()?;
    if label != "PUBLIC KEY" {
        return None;
    }
    parse_x25519_public_der(document.as_bytes())
}

fn parse_x25519_public_der(bytes: &[u8]) -> Option<x25519_dalek::PublicKey> {
    use pkcs8::der::Decode;
    let spki = pkcs8::SubjectPublicKeyInfoRef::from_der(bytes).ok()?;
    if spki.algorithm.oid != X25519_ALGORITHM_ID.oid {
        return None;
    }
    let raw = spki.subject_public_key.as_bytes()?;
    let bytes: [u8; 32] = raw.try_into().ok()?;
    Some(x25519_dalek::PublicKey::from(bytes))
}

/// Ed448's OID (RFC 8410 -- `id-Ed448`). `ed448-goldilocks`'s own
/// `SigningKey`/`VerifyingKey` *do* implement `pkcs8`'s encode/decode
/// traits, but only against a separate, incompatible v0.11 `pkcs8`
/// instance (`elliptic-curve` 0.14's own dependency -- required
/// unconditionally by `ed448-goldilocks` itself, not just behind a
/// feature flag, so there is no way to avoid a second `pkcs8`/
/// `elliptic-curve`/`signature` version family existing alongside this
/// file's own v0.10 ones while using this crate at all). Reusing that
/// v0.11 trait surface directly turned out to be a real dead end (its
/// own blanket `DecodePublicKey`/`DecodePrivateKey` impls didn't
/// actually resolve against `SigningKey`/`VerifyingKey`'s narrower
/// `TryFrom<...Ref<'_>>` impls in this pre-release version -- a rough
/// edge consistent with it not being a stable release yet). So this
/// hand-builds the PKCS8/SPKI wrapping instead, exactly the way
/// X25519's own `x25519_private_to_pem`/`parse_x25519_private_der`
/// above already do for the same RFC 8410 OKP-key shape -- same
/// nested-OCTET-STRING private-key layout, same flat-bytes SPKI
/// `BIT STRING` public-key layout, just Ed448's OID and 57-byte (not
/// 32-byte) raw key length.
const ED448_ALGORITHM_ID: pkcs8::AlgorithmIdentifierRef<'static> = pkcs8::AlgorithmIdentifierRef {
    oid: pkcs8::ObjectIdentifier::new_unwrap("1.3.101.113"),
    parameters: None,
};

fn ed448_private_to_pem(key: &ed448_goldilocks::SigningKey) -> Option<String> {
    let seed = key.as_bytes();
    let mut private_key = [0u8; 59];
    private_key[0] = 0x04;
    private_key[1] = 0x39;
    private_key[2..].copy_from_slice(seed.as_slice());
    let public_key = key.verifying_key().to_bytes();
    let info = pkcs8::PrivateKeyInfo {
        algorithm: ED448_ALGORITHM_ID,
        private_key: &private_key,
        public_key: Some(&public_key),
    };
    let document = pkcs8::SecretDocument::encode_msg(&info).ok()?;
    document
        .to_pem("PRIVATE KEY", pkcs8::LineEnding::LF)
        .ok()
        .map(|pem| pem.to_string())
}

fn ed448_public_to_pem(key: &ed448_goldilocks::VerifyingKey) -> Option<String> {
    let bytes = key.to_bytes();
    let spki = pkcs8::SubjectPublicKeyInfoRef {
        algorithm: ED448_ALGORITHM_ID,
        subject_public_key: pkcs8::der::asn1::BitStringRef::new(0, &bytes).ok()?,
    };
    let document: pkcs8::Document = spki.try_into().ok()?;
    document.to_pem("PUBLIC KEY", pkcs8::LineEnding::LF).ok()
}

fn parse_ed448_private_pem(pem: &str) -> Option<ed448_goldilocks::SigningKey> {
    let (label, document) = pkcs8::SecretDocument::from_pem(pem).ok()?;
    if label != "PRIVATE KEY" {
        return None;
    }
    parse_ed448_private_der(document.as_bytes())
}

fn parse_ed448_private_der(bytes: &[u8]) -> Option<ed448_goldilocks::SigningKey> {
    use pkcs8::der::Decode;
    let info = pkcs8::PrivateKeyInfo::from_der(bytes).ok()?;
    if info.algorithm.oid != ED448_ALGORITHM_ID.oid {
        return None;
    }
    let seed: &[u8] = match info.private_key {
        [0x04, 0x39, rest @ ..] if rest.len() == 57 => rest,
        _ => return None,
    };
    ed448_goldilocks::SigningKey::try_from(seed).ok()
}

fn parse_ed448_private_encrypted_pem(
    pem: &str,
    passphrase: &str,
) -> Option<ed448_goldilocks::SigningKey> {
    let (label, encrypted) = pkcs8::Document::from_pem(pem).ok()?;
    if label != "ENCRYPTED PRIVATE KEY" {
        return None;
    }
    parse_ed448_private_encrypted_der(encrypted.as_bytes(), passphrase)
}

fn parse_ed448_private_encrypted_der(
    bytes: &[u8],
    passphrase: &str,
) -> Option<ed448_goldilocks::SigningKey> {
    let document = pkcs8::EncryptedPrivateKeyInfo::try_from(bytes)
        .ok()?
        .decrypt(passphrase)
        .ok()?;
    parse_ed448_private_der(document.as_bytes())
}

fn parse_ed448_public_pem(pem: &str) -> Option<ed448_goldilocks::VerifyingKey> {
    let (label, document) = pkcs8::Document::from_pem(pem).ok()?;
    if label != "PUBLIC KEY" {
        return None;
    }
    parse_ed448_public_der(document.as_bytes())
}

fn parse_ed448_public_der(bytes: &[u8]) -> Option<ed448_goldilocks::VerifyingKey> {
    use pkcs8::der::Decode;
    let spki = pkcs8::SubjectPublicKeyInfoRef::from_der(bytes).ok()?;
    if spki.algorithm.oid != ED448_ALGORITHM_ID.oid {
        return None;
    }
    let raw = spki.subject_public_key.as_bytes()?;
    let bytes: [u8; 57] = raw.try_into().ok()?;
    ed448_goldilocks::VerifyingKey::from_bytes(&bytes).ok()
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
    if let Some(key) = parse_ed448_private_pem(pem) {
        return Some(ParsedKey::Ed448Private(Box::new(key)));
    }
    if let Some(key) = parse_x25519_private_pem(pem) {
        return Some(ParsedKey::X25519Private(Box::new(key)));
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
    if let Some(key) = parse_ed448_public_pem(pem) {
        return Some(ParsedKey::Ed448Public(Box::new(key)));
    }
    if let Some(key) = parse_x25519_public_pem(pem) {
        return Some(ParsedKey::X25519Public(Box::new(key)));
    }
    None
}

/// Tries every supported private key shape that needs a `passphrase`
/// (the modern PKCS8 `ENCRYPTED PRIVATE KEY` format only -- see this
/// module's own doc comment) in `is_der`'s format.
fn parse_key_bytes_encrypted(bytes: &[u8], is_der: bool, passphrase: &str) -> Option<ParsedKey> {
    if is_der {
        if let Ok(key) = RsaPrivateKey::from_pkcs8_encrypted_der(bytes, passphrase) {
            return Some(ParsedKey::RsaPrivate(Box::new(key)));
        }
        if let Ok(key) = p256::ecdsa::SigningKey::from_pkcs8_encrypted_der(bytes, passphrase) {
            return Some(ParsedKey::EcPrivateP256(Box::new(key)));
        }
        if let Ok(key) = p384::ecdsa::SigningKey::from_pkcs8_encrypted_der(bytes, passphrase) {
            return Some(ParsedKey::EcPrivateP384(Box::new(key)));
        }
        if let Some(key) = parse_ec_p521_private_encrypted_der(bytes, passphrase) {
            return Some(ParsedKey::EcPrivateP521(Box::new(key)));
        }
        if let Ok(key) = ed25519_dalek::SigningKey::from_pkcs8_encrypted_der(bytes, passphrase) {
            return Some(ParsedKey::Ed25519Private(Box::new(key)));
        }
        if let Some(key) = parse_ed448_private_encrypted_der(bytes, passphrase) {
            return Some(ParsedKey::Ed448Private(Box::new(key)));
        }
        if let Some(key) = parse_x25519_private_encrypted_der(bytes, passphrase) {
            return Some(ParsedKey::X25519Private(Box::new(key)));
        }
        None
    } else {
        let text = std::str::from_utf8(bytes).ok()?;
        if let Ok(key) = RsaPrivateKey::from_pkcs8_encrypted_pem(text, passphrase) {
            return Some(ParsedKey::RsaPrivate(Box::new(key)));
        }
        if let Ok(key) = p256::ecdsa::SigningKey::from_pkcs8_encrypted_pem(text, passphrase) {
            return Some(ParsedKey::EcPrivateP256(Box::new(key)));
        }
        if let Ok(key) = p384::ecdsa::SigningKey::from_pkcs8_encrypted_pem(text, passphrase) {
            return Some(ParsedKey::EcPrivateP384(Box::new(key)));
        }
        if let Some(key) = parse_ec_p521_private_encrypted_pem(text, passphrase) {
            return Some(ParsedKey::EcPrivateP521(Box::new(key)));
        }
        if let Ok(key) = ed25519_dalek::SigningKey::from_pkcs8_encrypted_pem(text, passphrase) {
            return Some(ParsedKey::Ed25519Private(Box::new(key)));
        }
        if let Some(key) = parse_ed448_private_encrypted_pem(text, passphrase) {
            return Some(ParsedKey::Ed448Private(Box::new(key)));
        }
        if let Some(key) = parse_x25519_private_encrypted_pem(text, passphrase) {
            return Some(ParsedKey::X25519Private(Box::new(key)));
        }
        None
    }
}

/// Tries every supported private-then-public key shape in DER form.
fn parse_key_der(bytes: &[u8]) -> Option<ParsedKey> {
    if let Ok(key) = RsaPrivateKey::from_pkcs8_der(bytes) {
        return Some(ParsedKey::RsaPrivate(Box::new(key)));
    }
    if let Ok(key) = RsaPrivateKey::from_pkcs1_der(bytes) {
        return Some(ParsedKey::RsaPrivate(Box::new(key)));
    }
    if let Ok(key) = p256::ecdsa::SigningKey::from_pkcs8_der(bytes) {
        return Some(ParsedKey::EcPrivateP256(Box::new(key)));
    }
    if let Ok(key) = p256::ecdsa::SigningKey::from_sec1_der(bytes) {
        return Some(ParsedKey::EcPrivateP256(Box::new(key)));
    }
    if let Ok(key) = p384::ecdsa::SigningKey::from_pkcs8_der(bytes) {
        return Some(ParsedKey::EcPrivateP384(Box::new(key)));
    }
    if let Ok(key) = p384::ecdsa::SigningKey::from_sec1_der(bytes) {
        return Some(ParsedKey::EcPrivateP384(Box::new(key)));
    }
    if let Some(key) = parse_ec_p521_private_der(bytes) {
        return Some(ParsedKey::EcPrivateP521(Box::new(key)));
    }
    if let Ok(key) = ed25519_dalek::SigningKey::from_pkcs8_der(bytes) {
        return Some(ParsedKey::Ed25519Private(Box::new(key)));
    }
    if let Some(key) = parse_ed448_private_der(bytes) {
        return Some(ParsedKey::Ed448Private(Box::new(key)));
    }
    if let Some(key) = parse_x25519_private_der(bytes) {
        return Some(ParsedKey::X25519Private(Box::new(key)));
    }
    if let Ok(key) = RsaPublicKey::from_public_key_der(bytes) {
        return Some(ParsedKey::RsaPublic(Box::new(key)));
    }
    if let Ok(key) = RsaPublicKey::from_pkcs1_der(bytes) {
        return Some(ParsedKey::RsaPublic(Box::new(key)));
    }
    if let Ok(key) = p256::ecdsa::VerifyingKey::from_public_key_der(bytes) {
        return Some(ParsedKey::EcPublicP256(Box::new(key)));
    }
    if let Ok(key) = p384::ecdsa::VerifyingKey::from_public_key_der(bytes) {
        return Some(ParsedKey::EcPublicP384(Box::new(key)));
    }
    if let Some(key) = parse_ec_p521_public_der(bytes) {
        return Some(ParsedKey::EcPublicP521(Box::new(key)));
    }
    if let Ok(key) = ed25519_dalek::VerifyingKey::from_public_key_der(bytes) {
        return Some(ParsedKey::Ed25519Public(Box::new(key)));
    }
    if let Some(key) = parse_ed448_public_der(bytes) {
        return Some(ParsedKey::Ed448Public(Box::new(key)));
    }
    if let Some(key) = parse_x25519_public_der(bytes) {
        return Some(ParsedKey::X25519Public(Box::new(key)));
    }
    None
}

/// The legacy OpenSSL PEM encryption format ("traditional" PKCS#1 RSA/
/// SEC1 EC private keys, `openssl {rsa,ec} -traditional -des3`/etc. --
/// predates PKCS#8's own, unrelated `ENCRYPTED PRIVATE KEY` format,
/// which this module already supports via `pkcs8`'s "encryption"
/// feature): a plain, un-armored PEM block (`-----BEGIN RSA PRIVATE
/// KEY-----`/`-----BEGIN EC PRIVATE KEY-----`, not the PKCS#8 header)
/// with two extra header lines before the base64 body:
/// ```text
/// Proc-Type: 4,ENCRYPTED
/// DEK-Info: <cipher-name>,<hex-iv>
/// ```
/// The encryption key itself is derived from the passphrase with
/// OpenSSL's own legacy (non-PBKDF2) `EVP_BytesToKey`: repeatedly
/// hashing `MD5(previous_digest || passphrase || salt)` (the `DEK-
/// Info` IV's first 8 bytes double as the salt) until enough key bytes
/// exist, then decrypting the body with that cipher/IV directly (no
/// separate KDF salt/iteration count anywhere in the file, unlike
/// PKCS#8's own PBES2 -- this format predates it). Real Node itself
/// does *not* support this format either (`ERR_OSSL_UNSUPPORTED` since
/// Node stopped linking against OpenSSL's legacy provider by default),
/// so there is no real-Node behavior to match here -- this is purely
/// "make real, pre-existing OpenSSL key files usable," cross-checked
/// against what real OpenSSL 3.5 itself produces and can decrypt back.
fn legacy_evp_bytes_to_key_md5(passphrase: &[u8], salt: &[u8], key_len: usize) -> Vec<u8> {
    let mut key = Vec::with_capacity(key_len);
    let mut previous: Vec<u8> = Vec::new();
    while key.len() < key_len {
        let mut round = Vec::with_capacity(previous.len() + passphrase.len() + salt.len());
        round.extend_from_slice(&previous);
        round.extend_from_slice(passphrase);
        round.extend_from_slice(salt);
        previous = md5::Md5::digest(&round).to_vec();
        key.extend_from_slice(&previous);
    }
    key.truncate(key_len);
    key
}

fn legacy_pem_decrypt_cbc(
    cipher: &str,
    key: &[u8],
    iv: &[u8],
    ciphertext: &[u8],
) -> Option<Vec<u8>> {
    use cbc::cipher::{BlockDecryptMut, KeyIvInit};
    macro_rules! decrypt_with {
        ($block:ty) => {
            cbc::Decryptor::<$block>::new_from_slices(key, iv)
                .ok()?
                .decrypt_padded_vec_mut::<cbc::cipher::block_padding::Pkcs7>(ciphertext)
                .ok()
        };
    }
    match cipher {
        "DES-EDE3-CBC" => decrypt_with!(des::TdesEde3),
        "AES-128-CBC" => decrypt_with!(aes::Aes128),
        "AES-192-CBC" => decrypt_with!(aes::Aes192),
        "AES-256-CBC" => decrypt_with!(aes::Aes256),
        _ => None,
    }
}

/// Parses a legacy-encrypted PEM's own header lines, derives the key,
/// decrypts the body, and hands back `(label, decrypted DER bytes)` --
/// e.g. `("RSA PRIVATE KEY", <plain PKCS#1 DER>)` -- for the caller to
/// feed into the existing (unencrypted) PKCS1/SEC1 parsers below.
fn parse_legacy_encrypted_pem(pem: &str, passphrase: &str) -> Option<(String, Vec<u8>)> {
    let mut lines = pem.lines();
    let label = lines
        .next()?
        .trim()
        .strip_prefix("-----BEGIN ")?
        .strip_suffix("-----")?
        .to_string();
    if !matches!(label.as_str(), "RSA PRIVATE KEY" | "EC PRIVATE KEY") {
        return None;
    }
    let mut dek_info = None;
    let mut body = String::new();
    let mut in_body = false;
    for line in lines {
        let trimmed = line.trim();
        if trimmed.starts_with("-----END ") {
            break;
        }
        if !in_body {
            if trimmed.is_empty() {
                in_body = true;
            } else if let Some(rest) = trimmed.strip_prefix("DEK-Info: ") {
                dek_info = Some(rest.to_string());
            }
            continue;
        }
        body.push_str(trimmed);
    }
    let dek_info = dek_info?;
    let (cipher, iv_hex) = dek_info.split_once(',')?;
    let iv = hex_decode(iv_hex);
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(body)
        .ok()?;
    let key_len = match cipher {
        "DES-EDE3-CBC" => 24,
        "AES-128-CBC" => 16,
        "AES-192-CBC" => 24,
        "AES-256-CBC" => 32,
        _ => return None,
    };
    let salt = iv.get(..8)?;
    let key = legacy_evp_bytes_to_key_md5(passphrase.as_bytes(), salt, key_len);
    let plaintext = legacy_pem_decrypt_cbc(cipher, &key, &iv, &ciphertext)?;
    Some((label, plaintext))
}

fn parse_legacy_encrypted_key(pem: &str, passphrase: &str) -> Option<ParsedKey> {
    let (label, der) = parse_legacy_encrypted_pem(pem, passphrase)?;
    match label.as_str() {
        "RSA PRIVATE KEY" => RsaPrivateKey::from_pkcs1_der(&der)
            .ok()
            .map(|key| ParsedKey::RsaPrivate(Box::new(key))),
        "EC PRIVATE KEY" => {
            if let Ok(key) = p256::ecdsa::SigningKey::from_sec1_der(&der) {
                return Some(ParsedKey::EcPrivateP256(Box::new(key)));
            }
            if let Ok(key) = p384::ecdsa::SigningKey::from_sec1_der(&der) {
                return Some(ParsedKey::EcPrivateP384(Box::new(key)));
            }
            parse_ec_p521_private_der(&der).map(|key| ParsedKey::EcPrivateP521(Box::new(key)))
        }
        _ => None,
    }
}

/// The single entry point for resolving *any* key material shape this
/// module supports: PEM or DER, passphrase-protected or not. Tries an
/// encrypted-private-key parse first when `passphrase` is non-empty
/// (a passphrase only ever makes sense for a private key, and a real
/// wrong-shape input still correctly falls through to `None` rather
/// than silently ignoring the passphrase), then the unencrypted
/// private-then-public cascade in the requested format.
fn parse_key_bytes(bytes: &[u8], is_der: bool, passphrase: &str) -> Option<ParsedKey> {
    if !passphrase.is_empty() {
        if let Some(key) = parse_key_bytes_encrypted(bytes, is_der, passphrase) {
            return Some(key);
        }
        if !is_der {
            if let Some(key) = std::str::from_utf8(bytes)
                .ok()
                .and_then(|pem| parse_legacy_encrypted_key(pem, passphrase))
            {
                return Some(key);
            }
        }
    }
    if is_der {
        parse_key_der(bytes)
    } else {
        parse_key(std::str::from_utf8(bytes).ok()?)
    }
}

/// Backs `createPrivateKey`/`createPublicKey`/`Sign.sign`/`Verify.
/// verify` (via `parseAsymmetricKeyMaterial`, which every one of those
/// funnels through): resolves PEM/DER/passphrase-protected key
/// material and re-encodes it as a plain, unencrypted PKCS8/SPKI PEM
/// string -- the *only* thing every other function in this module
/// (`crypto_asymmetric_sign_hex`/`_verify`/`_pss`/encrypt/decrypt) ever
/// sees, so none of them need their own DER/passphrase handling.
pub(crate) fn crypto_import_key_json(bytes_hex: &str, is_der: bool, passphrase: &str) -> String {
    let bytes = hex_decode(bytes_hex);
    let Some(key) = parse_key_bytes(&bytes, is_der, passphrase) else {
        return r#"{"valid":false}"#.to_string();
    };
    let Some(pem) = key.to_pem() else {
        return r#"{"valid":false}"#.to_string();
    };
    let (modulus_length, public_exponent) = match key.rsa_details() {
        Some((bits, exponent)) => (Some(bits), Some(exponent)),
        None => (None, None),
    };
    serde_json::json!({
        "valid": true,
        "pem": pem,
        "keyType": key.key_type(),
        "isPrivate": key.is_private(),
        "namedCurve": key.curve(),
        "modulusLength": modulus_length,
        "publicExponent": public_exponent,
    })
    .to_string()
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
        ParsedKey::Ed448Private(key) => Ok(key.sign_raw(data).to_bytes().to_vec()),
        ParsedKey::RsaPublic(_)
        | ParsedKey::EcPublicP256(_)
        | ParsedKey::EcPublicP384(_)
        | ParsedKey::EcPublicP521(_)
        | ParsedKey::Ed25519Public(_)
        | ParsedKey::Ed448Public(_) => Err("a public key cannot sign".to_string()),
        ParsedKey::X25519Private(_) | ParsedKey::X25519Public(_) => {
            Err("an X25519 key cannot sign -- it's a Diffie-Hellman key-agreement key, not a signing key".to_string())
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
        ParsedKey::Ed448Private(key) => ed448_goldilocks::Signature::from_slice(signature)
            .is_ok_and(|sig| key.verifying_key().verify_raw(&sig, data).is_ok()),
        ParsedKey::Ed448Public(key) => ed448_goldilocks::Signature::from_slice(signature)
            .is_ok_and(|sig| key.verify_raw(&sig, data).is_ok()),
        ParsedKey::X25519Private(_) | ParsedKey::X25519Public(_) => return Err(
            "an X25519 key cannot verify -- it's a Diffie-Hellman key-agreement key, not a signing key".to_string()
        ),
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
