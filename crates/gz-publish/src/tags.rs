//! Tags and titles of a release.
//!
//! Tags this project deleted and when, one per line (`<tag>\t<unix seconds>`), live in
//! `<project>\releases\deleted.tags` - a folder no build, package or content view ever touches. A
//! release deleted and published again under the same tag keeps its download address, and GitHub
//! goes on answering that address with the deleted files for minutes (seen live: still stale after
//! 40 s, fresh after 8 min). So a tag deleted within the last hour is not offered again; under a
//! fresh tag the address is new.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How long a deleted tag rests.
pub const TAG_REST_SECONDS: i64 = 3600;

/// `<project>\releases\deleted.tags`.
#[must_use]
pub fn deleted_tags_file(project_root: &Path) -> PathBuf {
    project_root.join("releases").join("deleted.tags")
}

/// Seconds since 1970.
#[must_use]
pub fn now_seconds() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// `_atoi64`: optional blanks and sign, then digits.
fn atoi64(s: &str) -> i64 {
    let s = s.trim_start_matches([' ', '\t']);
    let (neg, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, s.get(1..).unwrap_or("")),
        Some(b'+') => (false, s.get(1..).unwrap_or("")),
        _ => (false, s),
    };
    let n = digits
        .bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0i64, |acc, d| acc.saturating_mul(10).saturating_add(i64::from(d - b'0')));
    if neg { -n } else { n }
}

/// Whether the file's text names `tag` as deleted less than an hour before `now`.
#[must_use]
pub fn tag_just_deleted(deleted: &str, tag: &str, now: i64) -> bool {
    deleted.split('\n').any(|line| {
        let Some((name, at)) = line.split_once('\t') else { return false };
        name.eq_ignore_ascii_case(tag) && now - atoi64(at) < TAG_REST_SECONDS
    })
}

/// `v1.0.0` - or `v1.0.0-r2`, `-r3`, ... while the plainer ones were deleted a moment ago.
#[must_use]
pub fn fresh_tag(version: &str, deleted: &str, now: i64) -> String {
    let base = format!("v{version}");
    let mut tag = base.clone();
    let mut n = 2;
    while n < 1000 && tag_just_deleted(deleted, &tag, now) {
        tag = format!("{base}-r{n}");
        n += 1;
    }
    tag
}

/// A git tag name, kept to what every tool accepts.
#[must_use]
pub fn valid_tag(tag: &str) -> bool {
    let n = tag.len();
    if n == 0
        || n > 100
        || tag.starts_with('-')
        || tag.starts_with('.')
        || tag.ends_with('.')
        || tag.contains("..")
    {
        return false;
    }
    tag.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
}

/// `<name or id> <version>`.
#[must_use]
pub fn default_title(id: &str, name: &str, version: &str) -> String {
    format!("{} {version}", if name.is_empty() { id } else { name })
}

/// Notes a deleted tag. The file is named by the request: the project may have moved on by now.
pub fn note_deleted_tag(tags_file: &Path, tag: &str) {
    if tags_file.as_os_str().is_empty() || tag.is_empty() {
        return;
    }
    if let Some(folder) = tags_file.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(tags_file) {
        let _ = writeln!(f, "{tag}\t{}", now_seconds());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_tags() {
        let now = 1_800_000_000;
        assert_eq!(fresh_tag("1.0.0", "", now), "v1.0.0");
        // deleted a minute ago: the next free one
        let recent = format!("v1.0.0\t{}\nV1.0.0-R2\t{}\n", now - 60, now - 120);
        assert_eq!(fresh_tag("1.0.0", &recent, now), "v1.0.0-r3");
        // deleted over an hour ago: offered again
        let old = format!("v1.0.0\t{}\n", now - TAG_REST_SECONDS);
        assert_eq!(fresh_tag("1.0.0", &old, now), "v1.0.0");
        // another version's deletion does not count, nor does a line without a time
        assert_eq!(fresh_tag("1.0.1", &recent, now), "v1.0.1");
        assert!(!tag_just_deleted("v1.0.0\n", "v1.0.0", now));
        // a line with Windows line ends still reads
        assert!(tag_just_deleted(&format!("v2.0.0\t{}\r\n", now - 5), "v2.0.0", now));
    }

    #[test]
    fn tags_every_tool_takes() {
        assert!(valid_tag("v1.0.0-r2"));
        assert!(valid_tag("release_1"));
        assert!(!valid_tag("bad tag"));
        assert!(!valid_tag("-v1"));
        assert!(!valid_tag(".v1"));
        assert!(!valid_tag("v1."));
        assert!(!valid_tag("v1..0"));
        assert!(!valid_tag(""));
        assert!(!valid_tag(&"a".repeat(101)));
        assert_eq!(default_title("od", "", "1.0.0"), "od 1.0.0");
        assert_eq!(default_title("od", "Overnight Delivery", "1.0.0"), "Overnight Delivery 1.0.0");
    }
}
