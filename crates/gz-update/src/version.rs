//! A release's version: a strict `MAJOR.MINOR.PATCH`, as the tag and the manifest spell it.

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// `1.2.3`; no `v`, no pre-release or build part, no leading zero.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let mut next = || {
            let p = parts.next()?;
            let strict = !p.is_empty()
                && p.len() <= 9
                && p.bytes().all(|b| b.is_ascii_digit())
                && (p == "0" || !p.starts_with('0'));
            if strict { p.parse().ok() } else { None }
        };
        let v = Self { major: next()?, minor: next()?, patch: next()? };
        parts.next().is_none().then_some(v)
    }

    /// The version these programs were built as.
    #[must_use]
    pub fn compiled() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).unwrap_or(Self { major: 0, minor: 0, patch: 0 })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::Version;

    #[test]
    fn only_a_strict_version_is_one() {
        let v = |s| Version::parse(s);
        assert_eq!(v("0.7.1"), Some(Version { major: 0, minor: 7, patch: 1 }));
        assert_eq!(v("10.0.12").map(|x| x.to_string()), Some("10.0.12".to_owned()));
        for bad in ["v0.7.1", "0.7", "0.7.1.2", "0.07.1", "0.7.1-rc1", "", "0..1", " 0.7.1"] {
            assert_eq!(v(bad), None, "{bad}");
        }
        assert!(v("0.10.0") > v("0.9.9"));
        assert!(v("1.0.0") > v("0.99.99"));
    }
}
