//! The key file (MOD_CATALOG 4.1), byte-compatible both ways with the catalog's
//! `tools/xms_catalog.py`:
//!
//! ```text
//! xms-key 1
//! public  = p256:...
//! kdf     = pbkdf2-sha256, 600000, <base64url salt>
//! cipher  = hmac-sha256-ctr, <base64url nonce>
//! private = <base64url of the encrypted scalar>
//! mac     = <base64url of the tag>
//! ```
//!
//! or, unprotected, `public` and `private` (the plain scalar) only. The messages are the catalog's
//! own.

use zeroize::{Zeroize, Zeroizing};

use crate::b64;
use crate::crypto::{EcKey, PublicPoint, hmac, pbkdf2_64, random, stream};

/// The rounds a new protected key file is written with.
pub const KDF_ROUNDS: u32 = 600_000;
/// The largest key file read.
pub const KEY_FILE_LIMIT: u64 = 16 << 10;

/// A key file as read: the private part still sealed when it is protected.
pub struct KeyFile {
    pub public_text: String,
    private_raw: Zeroizing<Vec<u8>>,
    pub protected: bool,
    rounds: u32,
    salt: Vec<u8>,
    nonce: Vec<u8>,
    mac: Vec<u8>,
}

impl std::fmt::Debug for KeyFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeyFile")
            .field("public", &self.public_text)
            .field("protected", &self.protected)
            .finish_non_exhaustive()
    }
}

/// `name = value` of the first line whose key is `name` (`Field`): the key loses trailing
/// blanks, the value its leading blanks and trailing spaces and CR.
#[must_use]
pub fn field<'a>(text: &'a str, name: &str) -> &'a str {
    for line in text.split('\n') {
        let Some(eq) = line.find('=') else { continue };
        let (key, value) = (line.get(..eq).unwrap_or(""), line.get(eq + 1..).unwrap_or(""));
        if key.trim_end_matches([' ', '\t']) == name {
            return value.trim_start_matches([' ', '\t']).trim_end_matches([' ', '\r']);
        }
    }
    ""
}

/// Reads the text of a key file.
pub fn parse(text: &str) -> Result<KeyFile, String> {
    if !text.starts_with("xms-key 1") {
        // the article is the catalog's text's
        return Err("not an Grizzly key file".to_owned());
    }
    let public_text = field(text, "public").to_owned();
    if !public_text.starts_with("p256:") {
        return Err("the key file names no public key".to_owned());
    }
    let private_raw = Zeroizing::new(b64::decode(field(text, "private")).unwrap_or_default());
    if private_raw.len() != 32 {
        return Err("the key file holds no private key".to_owned());
    }
    let mut file = KeyFile {
        public_text,
        private_raw,
        protected: false,
        rounds: 0,
        salt: Vec::new(),
        nonce: Vec::new(),
        mac: Vec::new(),
    };
    let kdf = field(text, "kdf");
    file.protected = !kdf.is_empty();
    if !file.protected {
        return Ok(file);
    }
    let parts: Vec<&str> = kdf.split(',').map(|p| p.trim_start_matches(' ')).collect();
    let cipher = field(text, "cipher");
    let comma = cipher.find(',');
    let known = parts.len() == 3
        && parts.first() == Some(&"pbkdf2-sha256")
        && comma.is_some_and(|c| cipher.get(..c) == Some("hmac-sha256-ctr"));
    if !known {
        return Err("the key file is protected in a way this editor does not know".to_owned());
    }
    file.rounds = parts.get(1).map_or(0, |r| leading_number(r));
    let nonce = comma.and_then(|c| cipher.get(c + 1..)).unwrap_or("").trim_start_matches(' ');
    let damaged = || "the key file's protection is damaged".to_owned();
    file.salt = parts.get(2).and_then(|s| b64::decode(s)).ok_or_else(damaged)?;
    file.nonce = b64::decode(nonce).ok_or_else(damaged)?;
    file.mac = b64::decode(field(text, "mac")).ok_or_else(damaged)?;
    if file.rounds == 0 || file.mac.len() != 32 {
        return Err(damaged());
    }
    Ok(file)
}

/// `strtoul`: the digits a text starts with, 0 for none.
fn leading_number(text: &str) -> u32 {
    let digits = text
        .trim_start()
        .bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0u64, |n, d| (n * 10 + u64::from(d - b'0')).min(u64::from(u32::MAX)));
    digits as u32
}

impl KeyFile {
    /// The public point the file names; `None` when it is damaged.
    #[must_use]
    pub fn public(&self) -> Option<PublicPoint> {
        PublicPoint::parse(&self.public_text)
    }

    /// Opens the private key: the tag is checked before anything is decrypted,
    /// and the scalar must give the public point the file names.
    pub fn open(&self, password: &str) -> Result<EcKey, String> {
        let point = self.public().ok_or_else(|| "the key file's public key is damaged".to_owned())?;
        let mut scalar = Zeroizing::new([0u8; 32]);
        scalar.copy_from_slice(self.private_raw.as_slice());
        if self.protected {
            let derived = pbkdf2_64(password.as_bytes(), &self.salt, self.rounds);
            let (stream_key, tag_key) = derived.split_at(32);
            let tag = hmac(tag_key, &self.nonce, scalar.as_slice());
            if tag.as_slice() != self.mac.as_slice() {
                return Err("wrong password".to_owned());
            }
            stream(stream_key, &self.nonce, scalar.as_mut_slice());
        }
        let mismatch = || "the key file's private key does not match its public key".to_owned();
        let key = EcKey::from_scalar(&scalar).ok_or_else(mismatch)?;
        if !key.matches(&point) {
            return Err(mismatch());
        }
        Ok(key)
    }
}

/// The text of a new key file; `salt` and `nonce` are 16 random bytes each,
/// unused without a password. A new file always takes [`KDF_ROUNDS`]; a reader takes whatever
/// the file states.
#[must_use]
pub fn text_with(key: &EcKey, password: &str, salt: &[u8; 16], nonce: &[u8; 16], rounds: u32) -> String {
    let mut text = format!("xms-key 1\npublic  = {}\n", key.public.text());
    if password.is_empty() {
        text.push_str(&format!("private = {}\n", b64::encode(key.scalar())));
        return text;
    }
    let derived = pbkdf2_64(password.as_bytes(), salt, rounds);
    let (stream_key, tag_key) = derived.split_at(32);
    let mut encrypted = *key.scalar();
    stream(stream_key, nonce, &mut encrypted);
    let tag = hmac(tag_key, nonce, &encrypted);
    text.push_str(&format!("kdf     = pbkdf2-sha256, {rounds}, {}\n", b64::encode(salt)));
    text.push_str(&format!("cipher  = hmac-sha256-ctr, {}\n", b64::encode(nonce)));
    text.push_str(&format!("private = {}\n", b64::encode(&encrypted)));
    text.push_str(&format!("mac     = {}\n", b64::encode(&tag)));
    encrypted.zeroize();
    text
}

/// [`text_with`] with a fresh salt and nonce.
pub fn text(key: &EcKey, password: &str) -> Result<String, String> {
    let (mut salt, mut nonce) = ([0u8; 16], [0u8; 16]);
    random(&mut salt)?;
    random(&mut nonce)?;
    Ok(text_with(key, password, &salt, &nonce, KDF_ROUNDS))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn written_here_opens_here() {
        let key = EcKey::generate().unwrap();
        let full = text(&key, "pw").unwrap();
        assert!(field(&full, "kdf").starts_with("pbkdf2-sha256, 600000, "));
        // fewer rounds keep the test short; the file states them and the reader follows
        let protected = text_with(&key, "Пароль \"с\\кавычками\"", &[7; 16], &[9; 16], 1000);
        assert!(protected.starts_with("xms-key 1\npublic  = p256:"));
        assert_eq!(field(&protected, "kdf"), format!("pbkdf2-sha256, 1000, {}", b64::encode(&[7; 16])));
        assert!(field(&protected, "cipher").starts_with("hmac-sha256-ctr, "));
        let file = parse(&protected).unwrap();
        assert!(file.protected);
        let opened = file.open("Пароль \"с\\кавычками\"").unwrap();
        assert!(opened.matches(&key.public));
        assert_eq!(file.open("not the password").unwrap_err(), "wrong password");

        let plain = text(&key, "").unwrap();
        let file = parse(&plain).unwrap();
        assert!(!file.protected);
        assert!(file.open("").unwrap().matches(&key.public));
    }

    #[test]
    fn damaged_files() {
        assert_eq!(parse("xms-kez 1\n").unwrap_err(), "not an Grizzly key file");
        assert_eq!(parse("xms-key 1\npublic = x\n").unwrap_err(), "the key file names no public key");
        assert_eq!(
            parse("xms-key 1\npublic = p256:AA\nprivate = AA\n").unwrap_err(),
            "the key file holds no private key"
        );
        let key = EcKey::generate().unwrap();
        let good = text_with(&key, "pw", &[1; 16], &[2; 16], 1000);
        let odd = good.replace("hmac-sha256-ctr", "aes-ctr");
        assert_eq!(parse(&odd).unwrap_err(), "the key file is protected in a way this editor does not know");
        let mac = field(&good, "mac").to_owned();
        let cut = good.replace(&mac, "AAAA");
        assert_eq!(parse(&cut).unwrap_err(), "the key file's protection is damaged");
        // another key's public half
        let other = EcKey::generate().unwrap();
        let swapped = text(&key, "").unwrap().replace(&key.public.text(), &other.public.text());
        assert_eq!(
            parse(&swapped).unwrap().open("").unwrap_err(),
            "the key file's private key does not match its public key"
        );
        // CRLF and padded fields read the same
        let crlf = good.replace('\n', "\r\n");
        assert!(parse(&crlf).unwrap().open("pw").unwrap().matches(&key.public));
    }
}
