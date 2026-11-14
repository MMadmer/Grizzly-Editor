//! Path text the way the editor spells and compares it: backslashes, ASCII case ignored,
//! no trailing separator but a drive root's.
//!
//! Two kinds of normalisation live here and they are not interchangeable. [`full_path`] and
//! [`real_path`] ask Windows (the process's working directory, links resolved); they are for
//! comparing folders that exist. [`lexical_normalize`] works on the text alone; it is what the
//! content operations prove a path with, because a destination need not exist yet and a
//! relative path must never be read against the working directory.

use std::path::Path;

/// `X:\...` or `\\server\...`. `D:foo` is drive-relative and does not count.
#[must_use]
pub fn is_absolute(p: &str) -> bool {
    let b = p.as_bytes();
    let sep = |c: Option<&u8>| matches!(c, Some(b'\\' | b'/'));
    if sep(b.first()) && sep(b.get(1)) {
        return true;
    }
    b.first().is_some_and(u8::is_ascii_alphabetic) && b.get(1) == Some(&b':') && sep(b.get(2))
}

/// Forward slashes become backslashes.
#[must_use]
pub fn to_backslash(s: &str) -> String {
    s.replace('/', "\\")
}

/// Trailing separators off, but never below `X:\`.
#[must_use]
pub fn trim_trailing(mut s: String) -> String {
    while s.len() > 3 && (s.ends_with('\\') || s.ends_with('/')) {
        s.pop();
    }
    s
}

/// Separators, `.` and `..` resolved against the working directory; no
/// trailing separator but a drive root's. `None` for an empty or unusable path.
#[must_use]
pub fn full_path(path: &str) -> Option<String> {
    if path.is_empty() {
        return None;
    }
    let abs = std::path::absolute(Path::new(path)).ok()?;
    Some(trim_trailing(to_backslash(abs.to_str()?)))
}

/// Where a folder really is - links, junctions and 8.3 names resolved - so two spellings of one
/// place compare equal. `None` when it cannot be opened.
#[must_use]
pub fn real_path(path: &str) -> Option<String> {
    let real = std::fs::canonicalize(path).ok()?;
    let text = real.to_str()?;
    let out = if let Some(rest) = text.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{rest}")
    } else if let Some(rest) = text.strip_prefix("\\\\?\\") {
        rest.to_owned()
    } else {
        text.to_owned()
    };
    Some(trim_trailing(out))
}

/// Two spellings of one path, by their full forms.
#[must_use]
pub fn same_path(left: &str, right: &str) -> bool {
    match (full_path(left), full_path(right)) {
        (Some(l), Some(r)) => l.eq_ignore_ascii_case(&r),
        _ => false,
    }
}

/// `path` is `folder` or lies below it; both normalised.
#[must_use]
pub fn same_or_below(path: &str, folder: &str) -> bool {
    if path.is_empty() || folder.is_empty() || path.len() < folder.len() {
        return false;
    }
    let (Some(head), Some(rest)) = (path.get(..folder.len()), path.get(folder.len()..)) else { return false };
    if !head.eq_ignore_ascii_case(folder) {
        return false;
    }
    rest.is_empty() || rest.starts_with('\\') || folder.ends_with('\\')
}

/// The last segment of a backslashed path.
#[must_use]
pub fn leaf_of(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}

/// A path as the wire carries it: forward slashes, a double quote made a single one.
#[must_use]
pub fn wire(path: &str) -> String {
    path.chars()
        .map(|c| match c {
            '\\' => '/',
            '"' => '\'',
            c => c,
        })
        .collect()
}

/// The textual normalisation the content operations prove paths with: `/` becomes `\`,
/// repeats collapse, `.` goes and `..` pops. A `..` above the volume, a `:` or another
/// character no name may hold is `None`, never a silent clamp.
#[must_use]
pub fn lexical_normalize(input: &str) -> Option<String> {
    if !is_absolute(input) {
        return None;
    }
    let work = to_backslash(input);
    let (head, body) = if let Some(after) = work.strip_prefix("\\\\") {
        let slash = after.find('\\')?;
        let share = after.get(slash + 1..)?;
        if share.is_empty() {
            return None;
        }
        match share.find('\\') {
            Some(end) => {
                let cut = 2 + slash + 1 + end;
                (work.get(..cut)?.to_owned(), work.get(cut..)?.to_owned())
            }
            None => (work.clone(), String::new()),
        }
    } else {
        (work.get(..2)?.to_owned(), work.get(2..)?.to_owned())
    };

    let mut seg: Vec<&str> = Vec::new();
    for token in body.split('\\') {
        match token {
            "" | "." => {}
            ".." => {
                seg.pop()?;
            }
            name => {
                if name.contains([':', '<', '>', '"', '|', '?', '*']) {
                    return None;
                }
                seg.push(name);
            }
        }
    }
    let mut acc = head;
    for s in &seg {
        acc.push('\\');
        acc.push_str(s);
    }
    if seg.is_empty() {
        acc.push('\\');
    }
    Some(acc)
}

/// Prefix test on two normalised paths; the separator check keeps `<root>_backup` out of
/// `<root>`.
#[must_use]
pub fn is_inside(outer: &str, inner: &str) -> bool {
    let n = outer.len();
    if n == 0 || inner.len() < n {
        return false;
    }
    let (Some(head), Some(rest)) = (inner.get(..n), inner.get(n..)) else { return false };
    if !head.eq_ignore_ascii_case(outer) {
        return false;
    }
    outer.ends_with('\\') || rest.is_empty() || rest.starts_with('\\')
}

/// `a\b` from a directory and a leaf.
#[must_use]
pub fn join(dir: &str, leaf: &str) -> String {
    if dir.ends_with('\\') { format!("{dir}{leaf}") } else { format!("{dir}\\{leaf}") }
}

/// Whether `path` names the root of a drive or of a share: `X:\` or `\\server\share`.
#[must_use]
pub fn is_drive_root(path: &str) -> bool {
    let p = trim_trailing(to_backslash(path));
    let b = p.as_bytes();
    if b.len() <= 3 && b.first().is_some_and(u8::is_ascii_alphabetic) && b.get(1) == Some(&b':') {
        return true;
    }
    if let Some(after) = p.strip_prefix("\\\\") {
        let parts: Vec<&str> = after.split('\\').filter(|s| !s.is_empty()).collect();
        return parts.len() <= 2;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_needs_a_separator() {
        assert!(is_absolute("D:\\a"));
        assert!(is_absolute("d:/a"));
        assert!(is_absolute("\\\\server\\share"));
        assert!(!is_absolute("D:a"));
        assert!(!is_absolute("a\\b"));
        assert!(!is_absolute(""));
    }

    #[test]
    fn lexical_normalisation() {
        assert_eq!(lexical_normalize("D:/Games//Mod/./a/../b").as_deref(), Some("D:\\Games\\Mod\\b"));
        assert_eq!(lexical_normalize("D:\\").as_deref(), Some("D:\\"));
        assert_eq!(lexical_normalize("D:\\a\\..\\.."), None);
        assert_eq!(lexical_normalize("D:\\a\\b:stream"), None);
        assert_eq!(lexical_normalize("\\\\srv\\share\\x\\..\\y").as_deref(), Some("\\\\srv\\share\\y"));
        assert_eq!(lexical_normalize("\\\\srv"), None);
        assert_eq!(lexical_normalize("relative\\x"), None);
    }

    #[test]
    fn inside_means_whole_segments() {
        assert!(is_inside("D:\\Mod", "D:\\mod\\a"));
        assert!(is_inside("D:\\Mod", "D:\\Mod"));
        assert!(!is_inside("D:\\Mod", "D:\\Mod_backup"));
        assert!(is_inside("D:\\", "D:\\anything"));
        assert!(same_or_below("D:\\Mod\\x", "d:\\mod"));
        assert!(!same_or_below("D:\\Modx", "D:\\Mod"));
    }

    #[test]
    fn drive_roots() {
        assert!(is_drive_root("C:\\"));
        assert!(is_drive_root("c:"));
        assert!(is_drive_root("\\\\srv\\share\\"));
        assert!(!is_drive_root("C:\\Games"));
        assert!(!is_drive_root("\\\\srv\\share\\x"));
    }

    #[test]
    fn wire_form() {
        assert_eq!(wire("D:\\a\\\"b\""), "D:/a/'b'");
        assert_eq!(leaf_of("D:\\a\\Name"), "Name");
        assert_eq!(join("D:\\", "x"), "D:\\x");
        assert_eq!(join("D:\\a", "x"), "D:\\a\\x");
    }
}
