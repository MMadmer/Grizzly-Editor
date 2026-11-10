//! Signed text files (MOD_CATALOG 3.1): an ini text whose LAST section is
//!
//! ```ini
//! [signature]
//! key = p256:...
//! sig = 0123456789abcdef:...
//! ```
//!
//! The signed message is every byte before the `[signature]` header line, up to and including
//! the line break that ends the line before it.

use crate::b64;
use crate::crypto::{EcKey, PublicPoint, verify};

/// `body` followed by its `[signature]` section, in the body's own line ends (`Sign`): CRLF
/// when the body holds one, LF otherwise. A body that does not end with a line break gets one,
/// and the break is signed.
pub fn sign_text(body: &str, key: &EcKey) -> Result<String, String> {
    let nl = if body.contains("\r\n") { "\r\n" } else { "\n" };
    let mut signed = body.to_owned();
    if !signed.ends_with('\n') {
        signed.push_str(nl);
    }
    let signature = key.sign(signed.as_bytes())?;
    let (public, id) = (key.public.text(), key.public.id());
    signed.push_str(&format!("[signature]{nl}key = {public}{nl}sig = {id}:{}{nl}", b64::encode(&signature)));
    Ok(signed)
}

/// Why a text does not verify, and the key id its signature claims (empty when it names none).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifyError {
    pub signer: String,
    pub message: String,
}

fn fail(signer: &str, message: &str) -> VerifyError {
    VerifyError { signer: signer.to_owned(), message: message.to_owned() }
}

/// A signed text checked against the key it has to be signed with (`Verify`). `Ok` names the
/// signer's key id.
pub fn verify_text(text: &str, public_key: &str) -> Result<String, VerifyError> {
    let bytes = text.as_bytes();
    let mut header: Option<usize> = None;
    let mut at = 0usize;
    while at < bytes.len() {
        let end = text.get(at..).and_then(|t| t.find('\n')).map_or(bytes.len(), |n| at + n);
        let mut line_end = end;
        if line_end > at && bytes.get(line_end - 1) == Some(&b'\r') {
            line_end -= 1;
        }
        // the game and the catalog's tools do not take a header on the very first line either
        if at > 0 && text.get(at..line_end) == Some("[signature]") {
            if header.is_some() {
                return Err(fail("", "the text is signed twice"));
            }
            header = Some(at);
        }
        at = end + 1;
    }
    let Some(header) = header else { return Err(fail("", "not signed")) };

    let (mut key_text, mut sig_text) = (String::new(), String::new());
    let tail = text.get(header..).unwrap_or("");
    for line in tail.split('\n').skip(1) {
        let line = line.trim_end_matches(['\r', ' ', '\t']);
        if line.is_empty() {
            continue;
        }
        let (name, value) = match line.find('=') {
            Some(eq) => (line.get(..eq).unwrap_or(""), line.get(eq + 1..).unwrap_or("")),
            None => (line, ""),
        };
        let (name, value) = (name.trim_end_matches([' ', '\t']), value.trim_start_matches([' ', '\t']));
        if name == "key" && key_text.is_empty() {
            key_text = value.to_owned();
        } else if name == "sig" && sig_text.is_empty() {
            sig_text = value.to_owned();
        } else {
            return Err(fail("", "the signature section holds more than a key and a signature"));
        }
    }
    let Some(colon) = sig_text.find(':') else { return Err(fail("", "the signature is damaged")) };
    let signer = sig_text.get(..colon).unwrap_or("").to_owned();

    let Some(expected) = PublicPoint::parse(public_key) else { return Err(fail(&signer, "the expected key is damaged")) };
    if signer != expected.id() {
        return Err(VerifyError { message: format!("signed with another key ({signer})"), signer });
    }
    let raw = b64::decode(sig_text.get(colon + 1..).unwrap_or("")).unwrap_or_default();
    let Ok(signature) = <[u8; 64]>::try_from(raw.as_slice()) else { return Err(fail(&signer, "the signature is damaged")) };
    if !verify(&expected, bytes.get(..header).unwrap_or(&[]), &signature) {
        return Err(fail(&signer, "the signature does not match the text: it was changed after signing"));
    }
    Ok(signer)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn sign_then_verify() {
        let key = EcKey::generate().unwrap();
        let body = "; a release\r\n[release]\r\nid = od\r\n";
        let signed = sign_text(body, &key).unwrap();
        assert!(signed.starts_with(body));
        let tail = signed.get(body.len()..).unwrap();
        assert!(tail.starts_with("[signature]\r\nkey = p256:"));
        assert!(tail.contains(&format!("\r\nsig = {}:", key.public.id())));
        assert!(tail.ends_with("\r\n"));
        assert_eq!(verify_text(&signed, &key.public.text()).unwrap(), key.public.id());

        // one changed byte of the body breaks it
        let changed = signed.replacen("id = od", "id = oe", 1);
        assert_eq!(verify_text(&changed, &key.public.text()).unwrap_err().message, "the signature does not match the text: it was changed after signing");
        // another key is named as such
        let other = EcKey::generate().unwrap();
        let e = verify_text(&signed, &other.public.text()).unwrap_err();
        assert_eq!(e.signer, key.public.id());
        assert!(e.message.starts_with("signed with another key ("));
        // a body without a final line break gets one, in its own line ends
        let lf = sign_text("[a]\nb = 1", &key).unwrap();
        assert!(lf.starts_with("[a]\nb = 1\n[signature]\nkey = "));
        assert!(verify_text(&lf, &key.public.text()).is_ok());
        // anything after the section, or a second header, is refused
        assert!(verify_text(&format!("{signed}x = 1\r\n"), &key.public.text()).is_err());
        assert_eq!(verify_text(&format!("{signed}[signature]\r\n"), &key.public.text()).unwrap_err().message, "the text is signed twice");
        assert_eq!(verify_text(body, &key.public.text()).unwrap_err().message, "not signed");
    }
}
