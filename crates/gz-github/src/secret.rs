//! A token or a key: wiped from memory when the last copy goes, never printed.

use std::fmt;

use zeroize::Zeroizing;

/// A secret string. `Debug` and `Display` print no part of it; the bytes are zeroed on drop.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    #[must_use]
    pub fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    /// The value, for the one place that sends it.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Zeroizing<String>> for Secret {
    fn from(value: Zeroizing<String>) -> Self {
        Self(value)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.0.is_empty() { "Secret(empty)" } else { "Secret(<redacted>)" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_printed() {
        let s = Secret::new("ghp_abcdefghijklmnopqrstuvwxyz".to_owned());
        assert_eq!(format!("{s:?}"), "Secret(<redacted>)");
        assert_eq!(s.expose(), "ghp_abcdefghijklmnopqrstuvwxyz");
        assert_eq!(format!("{:?}", Secret::default()), "Secret(empty)");
    }
}
