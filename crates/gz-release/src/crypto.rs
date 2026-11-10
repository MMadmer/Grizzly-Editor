//! The primitives of MOD_CATALOG 3 and 4.1, pure Rust: SHA-256, HMAC-SHA256, PBKDF2-HMAC-SHA256,
//! the key file's HMAC stream, and ECDSA over P-256 with SHA-256.
//!
//! A signature differs in bytes from one Windows CNG makes for the same input - ECDSA picks a
//! nonce, here by RFC 6979 - and verifies all the same: the contract is verification.

use hmac::{Hmac, KeyInit, Mac};
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::b64;

/// SHA-256 of `data`.
#[must_use]
pub fn sha256(data: &[u8]) -> [u8; 32] {
    Sha256::digest(data).into()
}

/// Lower case hex SHA-256: the form every index, descriptor and card states.
#[must_use]
pub fn sha256_hex(data: &[u8]) -> String {
    b64::hex(&sha256(data))
}

/// A SHA-256 fed in pieces.
#[derive(Clone, Default)]
pub struct Sha256Stream(Sha256);

impl Sha256Stream {
    pub fn update(&mut self, data: &[u8]) {
        self.0.update(data);
    }

    #[must_use]
    pub fn finish_hex(self) -> String {
        let digest: [u8; 32] = self.0.finalize().into();
        b64::hex(&digest)
    }
}

/// HMAC-SHA256 over `a || b`.
#[must_use]
pub fn hmac(key: &[u8], a: &[u8], b: &[u8]) -> [u8; 32] {
    // HMAC takes a key of any length
    let Ok(mut mac) = <Hmac<Sha256> as KeyInit>::new_from_slice(key) else { return [0; 32] };
    mac.update(a);
    mac.update(b);
    mac.finalize().into_bytes().into()
}

/// PBKDF2-HMAC-SHA256 into 64 bytes: the stream key, then the tag key (4.1).
#[must_use]
pub fn pbkdf2_64(password: &[u8], salt: &[u8], rounds: u32) -> Zeroizing<[u8; 64]> {
    let mut out = Zeroizing::new([0u8; 64]);
    pbkdf2::pbkdf2_hmac::<Sha256>(password, salt, rounds, out.as_mut_slice());
    out
}

/// The key file's stream (4.1): block `i` is `HMAC-SHA256(key, nonce || i as 4 big-endian
/// bytes)`, XORed over `data`.
pub fn stream(key: &[u8], nonce: &[u8], data: &mut [u8]) {
    for (block, chunk) in data.chunks_mut(32).enumerate() {
        let counter = (block as u32).to_be_bytes();
        let mut pad = hmac(key, nonce, &counter);
        for (d, p) in chunk.iter_mut().zip(pad.iter()) {
            *d ^= p;
        }
        pad.zeroize();
    }
}

/// Fills `out` from the system's generator.
pub fn random(out: &mut [u8]) -> Result<(), String> {
    getrandom::fill(out).map_err(|e| format!("the system's random generator failed: {e}"))
}

/// A P-256 public point: the 64 bytes of `X || Y`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PublicPoint(pub [u8; 64]);

impl PublicPoint {
    /// `p256:<X||Y>` (3).
    #[must_use]
    pub fn text(&self) -> String {
        format!("p256:{}", b64::encode(&self.0))
    }

    /// The first 16 hex digits of the SHA-256 of `X || Y` (3).
    #[must_use]
    pub fn id(&self) -> String {
        let digest = sha256(&self.0);
        b64::hex(digest.get(..8).unwrap_or(&[]))
    }

    /// `p256:...` back into the point; `None` when it is not 64 bytes of base64url. Whether
    /// the point lies on the curve is asked by [`verify`].
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let raw = b64::decode(text.strip_prefix("p256:")?)?;
        <[u8; 64]>::try_from(raw.as_slice()).ok().map(Self)
    }

    fn verifying_key(&self) -> Option<VerifyingKey> {
        let mut sec1 = [0u8; 65];
        sec1[0] = 4;
        sec1[1..].copy_from_slice(&self.0);
        VerifyingKey::from_sec1_bytes(&sec1).ok()
    }
}

impl std::fmt::Debug for PublicPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text())
    }
}

/// A key with its private scalar. The scalar is wiped when the value goes; `Debug` shows the
/// public half only.
pub struct EcKey {
    pub public: PublicPoint,
    d: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for EcKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EcKey").field("public", &self.public).finish_non_exhaustive()
    }
}

fn public_of(sk: &SigningKey) -> Option<PublicPoint> {
    let point = sk.verifying_key().to_sec1_point(false);
    let bytes = point.as_bytes();
    let xy = bytes.get(1..65)?;
    <[u8; 64]>::try_from(xy).ok().map(PublicPoint)
}

impl EcKey {
    /// A new key from the system's generator.
    pub fn generate() -> Result<Self, String> {
        for _ in 0..64 {
            let mut d = Zeroizing::new([0u8; 32]);
            random(d.as_mut_slice())?;
            // a scalar of zero or not below the group order is drawn again
            if let Some(key) = Self::from_scalar(&d) {
                return Ok(key);
            }
        }
        Err("could not make a P-256 key".to_owned())
    }

    /// The key of a private scalar; `None` when it is no valid scalar.
    #[must_use]
    pub fn from_scalar(d: &[u8; 32]) -> Option<Self> {
        let sk = SigningKey::from_slice(d).ok()?;
        let public = public_of(&sk)?;
        Some(Self { public, d: Zeroizing::new(*d) })
    }

    /// The private scalar, for the key file.
    #[must_use]
    pub fn scalar(&self) -> &[u8; 32] {
        &self.d
    }

    /// Whether the scalar gives this public point (`KeyConsistent`).
    #[must_use]
    pub fn matches(&self, point: &PublicPoint) -> bool {
        self.public == *point
    }

    /// ECDSA over the SHA-256 of `data`: `r || s`.
    pub fn sign(&self, data: &[u8]) -> Result<[u8; 64], String> {
        let sk = SigningKey::from_slice(self.d.as_slice()).map_err(|_| "the author key is damaged".to_owned())?;
        let sig: Signature = sk.try_sign(data).map_err(|_| "the signature could not be made".to_owned())?;
        Ok(sig.to_bytes().into())
    }
}

/// ECDSA over the SHA-256 of `data` by `point`: false for a point off the curve or a signature
/// that does not hold.
#[must_use]
pub fn verify(point: &PublicPoint, data: &[u8], signature: &[u8; 64]) -> bool {
    let Some(vk) = point.verifying_key() else { return false };
    let Ok(sig) = Signature::from_slice(signature) else { return false };
    vk.verify(data, &sig).is_ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn known_answers() {
        assert_eq!(sha256_hex(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        let mut s = Sha256Stream::default();
        s.update(b"a");
        s.update(b"bc");
        assert_eq!(s.finish_hex(), sha256_hex(b"abc"));
        // RFC 4231, case 2
        assert_eq!(b64::hex(&hmac(b"Jefe", b"what do ya want ", b"for nothing?")), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
        // the pbkdf2 crate's own example, cut to 20 bytes
        let k = pbkdf2_64(b"password", b"salt", 1);
        assert_eq!(b64::hex(&k[..20]), "120fb6cffcf8b32c43e7225256c4f837a86548c9");
    }

    #[test]
    fn stream_is_its_own_inverse() {
        let mut data = *b"0123456789abcdef0123456789abcdef0123";
        stream(b"key", b"nonce", &mut data);
        assert_ne!(&data, b"0123456789abcdef0123456789abcdef0123");
        stream(b"key", b"nonce", &mut data);
        assert_eq!(&data, b"0123456789abcdef0123456789abcdef0123");
    }

    #[test]
    fn sign_then_verify() {
        let key = EcKey::generate().unwrap();
        let sig = key.sign(b"message").unwrap();
        assert!(verify(&key.public, b"message", &sig));
        assert!(!verify(&key.public, b"messagf", &sig));
        let other = EcKey::generate().unwrap();
        assert!(!verify(&other.public, b"message", &sig));
        assert_eq!(PublicPoint::parse(&key.public.text()), Some(key.public));
        assert_eq!(key.public.id().len(), 16);
        let again = EcKey::from_scalar(key.scalar()).unwrap();
        assert!(again.matches(&key.public));
        assert!(EcKey::from_scalar(&[0; 32]).is_none());
        // a point off the curve verifies nothing
        let off = PublicPoint([1; 64]);
        assert!(!verify(&off, b"message", &sig));
    }
}
