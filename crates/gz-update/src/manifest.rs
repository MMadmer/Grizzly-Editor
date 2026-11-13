//! `update-manifest.txt`: the managed set of a version (docs/UPDATES.md, section 1).
//!
//! ```text
//! schema=grizzly-editor.update/1
//! version=MAJOR.MINOR.PATCH
//! <sha256 hex><TAB><size><TAB><relative path, forward slashes>
//! ```

use std::collections::HashSet;

use crate::version::Version;

pub const SCHEMA: &str = "grizzly-editor.update/1";
/// The manifest's own name, at the installation's root and the archive's.
pub const MANIFEST: &str = "update-manifest.txt";
/// The updater's work folder at the installation's root.
pub const WORK: &str = ".update";

/// One managed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Lower-case hex.
    pub sha256: String,
    pub size: u64,
    /// Relative, forward slashes.
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub version: Version,
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ManifestError {
    /// A schema this client does not know: the release is offered as a page, not an update.
    UnknownSchema(String),
    /// What is wrong, as a sentence.
    Invalid(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSchema(s) => write!(f, "the manifest's schema '{s}' is not one this editor knows"),
            Self::Invalid(s) => f.write_str(s),
        }
    }
}

/// Why a manifest path cannot name a file of the installation, or `None` when it can.
#[must_use]
pub fn path_problem(path: &str) -> Option<&'static str> {
    if path.is_empty() {
        return Some("is empty");
    }
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return Some("is not a relative path with forward slashes");
    }
    if path.chars().any(|c| c.is_control() || matches!(c, '<' | '>' | '"' | '|' | '?' | '*')) {
        return Some("holds a character a file name cannot");
    }
    if path.split('/').any(|seg| seg.is_empty() || seg == "." || seg == ".." || seg.ends_with(['.', ' '])) {
        return Some("leaves the installation or names no file");
    }
    let lower = path.to_ascii_lowercase();
    if lower == MANIFEST || lower == WORK || lower.starts_with(".update/") {
        return Some("names the updater's own files");
    }
    None
}

/// The key two paths that name one file on Windows share.
#[must_use]
pub fn path_key(path: &str) -> String {
    path.to_lowercase()
}

/// Reads a manifest. Rows are refused when a path is absolute, has `..`, appears twice or names
/// the updater's own files; an unknown schema is an error of its own.
pub fn parse(bytes: &[u8]) -> Result<Manifest, ManifestError> {
    let bad = |s: String| ManifestError::Invalid(s);
    let text = std::str::from_utf8(bytes).map_err(|_| bad("the manifest is not UTF-8".to_owned()))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty());
    let schema = lines.next().and_then(|l| l.strip_prefix("schema=")).map(str::trim);
    match schema {
        Some(SCHEMA) => {}
        Some(other) => return Err(ManifestError::UnknownSchema(other.to_owned())),
        None => return Err(bad("the manifest does not start with its schema".to_owned())),
    }
    let mut version = None;
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for (n, line) in lines.enumerate() {
        if !line.contains('\t') {
            // a header line: version, or a key a later client may add
            if let Some(v) = line.strip_prefix("version=") {
                version = Some(Version::parse(v.trim()).ok_or_else(|| {
                    bad(format!("the manifest's version '{}' is not MAJOR.MINOR.PATCH", v.trim()))
                })?);
                continue;
            }
            if line.contains('=') && rows.is_empty() {
                continue;
            }
            return Err(bad(format!("line {} of the manifest is not a row", n + 2)));
        }
        let mut parts = line.splitn(3, '\t');
        let (sha, size, path) = (parts.next(), parts.next(), parts.next());
        let (Some(sha), Some(size), Some(path)) = (sha, size, path) else {
            return Err(bad(format!("line {} of the manifest is not a row", n + 2)));
        };
        let sha = sha.to_ascii_lowercase();
        if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad(format!("the row of {path} has no SHA-256")));
        }
        let size: u64 = size.trim().parse().map_err(|_| bad(format!("the row of {path} has no size")))?;
        if let Some(why) = path_problem(path) {
            return Err(bad(format!("the manifest path '{path}' {why}")));
        }
        if !seen.insert(path_key(path)) {
            return Err(bad(format!("the manifest names {path} twice")));
        }
        rows.push(Row { sha256: sha, size, path: path.to_owned() });
    }
    let version = version.ok_or_else(|| bad("the manifest names no version".to_owned()))?;
    Ok(Manifest { version, rows })
}

impl Manifest {
    /// The row of a path, ignoring case as Windows does.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&Row> {
        let key = path_key(path);
        self.rows.iter().find(|r| path_key(&r.path) == key)
    }

    /// The manifest's text: rows sorted by path.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut rows: Vec<&Row> = self.rows.iter().collect();
        rows.sort_by(|a, b| a.path.cmp(&b.path));
        let mut out = format!("schema={SCHEMA}\nversion={}\n", self.version);
        for r in rows {
            out.push_str(&format!("{}\t{}\t{}\n", r.sha256, r.size, r.path));
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const SHA: &str = "aa00000000000000000000000000000000000000000000000000000000000001";

    fn text(rows: &[&str]) -> String {
        let mut t = format!("schema={SCHEMA}\nversion=0.7.1\n");
        for r in rows {
            t.push_str(r);
            t.push('\n');
        }
        t
    }

    #[test]
    fn a_manifest_reads_and_writes_back() {
        let t =
            text(&[&format!("{SHA}\t12\teditor.bin"), &format!("{SHA}\t3\tgamedata/configs/nq/catalog.ltx")]);
        let m = parse(t.as_bytes()).unwrap();
        assert_eq!(m.version.to_string(), "0.7.1");
        assert_eq!(m.rows.len(), 2);
        assert_eq!(m.get("EDITOR.BIN").unwrap().size, 12);
        // sorted by path when written
        assert_eq!(
            m.to_text(),
            text(&[&format!("{SHA}\t12\teditor.bin"), &format!("{SHA}\t3\tgamedata/configs/nq/catalog.ltx")])
        );
        // CRLF line ends and a BOM read the same
        let crlf = format!("\u{feff}{}", t.replace('\n', "\r\n"));
        assert_eq!(parse(crlf.as_bytes()).unwrap(), m);
    }

    #[test]
    fn rows_that_would_leave_the_installation_are_refused() {
        for path in [
            "/abs.txt",
            "C:/x.txt",
            "a\\b.txt",
            "../up.txt",
            "a/../b.txt",
            "a//b.txt",
            "./a.txt",
            "update-manifest.txt",
            ".update/0.7.1/state.ltx",
            "a/b.",
            "a/b?.txt",
        ] {
            let err = parse(text(&[&format!("{SHA}\t1\t{path}")]).as_bytes()).unwrap_err();
            assert!(matches!(err, ManifestError::Invalid(_)), "{path}: {err:?}");
        }
        let twice = text(&[&format!("{SHA}\t1\ta.txt"), &format!("{SHA}\t1\tA.TXT")]);
        assert_eq!(
            parse(twice.as_bytes()).unwrap_err(),
            ManifestError::Invalid("the manifest names A.TXT twice".into())
        );
        assert!(parse(text(&["abc\t1\ta.txt"]).as_bytes()).is_err());
        assert!(parse(text(&[&format!("{SHA}\tx\ta.txt")]).as_bytes()).is_err());
        assert!(parse(b"schema=grizzly-editor.update/1\na.txt\n").is_err());
    }

    #[test]
    fn an_unknown_schema_is_its_own_answer() {
        assert_eq!(
            parse(b"schema=grizzly-editor.update/2\nversion=1.0.0\n").unwrap_err(),
            ManifestError::UnknownSchema("grizzly-editor.update/2".into())
        );
        assert!(matches!(parse(b"version=1.0.0\n").unwrap_err(), ManifestError::Invalid(_)));
        assert!(matches!(
            parse(format!("schema={SCHEMA}\n{SHA}\t1\ta.txt\n").as_bytes()).unwrap_err(),
            ManifestError::Invalid(_)
        ));
    }
}
