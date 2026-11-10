//! The ini text of the catalog's files (its index, `mods/<id>.ltx`) and of a release descriptor,
//! read the way `EditorCatalog::IniSection` reads it: trimmed lines, `;` starts a comment line, a
//! `;` outside quotes ends a value.

fn trim_spaces(s: &str) -> &str {
    s.trim_end_matches([' ', '\t', '\r']).trim_start_matches([' ', '\t'])
}

// a ';' outside quotes starts a comment
fn strip_comment(value: &str) -> &str {
    let mut quoted = false;
    for (i, c) in value.char_indices() {
        if c == '"' {
            quoted = !quoted;
        } else if c == ';' && !quoted {
            return trim_spaces(value.get(..i).unwrap_or(""));
        }
    }
    trim_spaces(value)
}

/// Every `key = value` of a section, in order.
#[must_use]
pub fn section<'a>(text: &'a str, name: &str) -> Vec<(&'a str, &'a str)> {
    let mut out = Vec::new();
    let mut inside = false;
    for raw in text.split('\n') {
        let line = trim_spaces(raw);
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let head = rest.find(']').map_or(rest, |close| rest.get(..close).unwrap_or(rest));
            inside = trim_spaces(head) == name;
            continue;
        }
        let Some(eq) = line.find('=') else { continue };
        if !inside {
            continue;
        }
        let key = trim_spaces(line.get(..eq).unwrap_or(""));
        let value = strip_comment(line.get(eq + 1..).unwrap_or(""));
        out.push((key, value));
    }
    out
}

/// The value of `key` in `section`, trimmed; `None` when it is not there.
#[must_use]
pub fn value<'a>(text: &'a str, sec: &str, key: &str) -> Option<&'a str> {
    section(text, sec).into_iter().find(|(k, _)| *k == key).map(|(_, v)| v)
}

/// A quoted card text, unescaped (`\n`, `\t`, `\"`, `\\`).
#[must_use]
pub fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() < 2 || bytes.first() != Some(&b'"') || bytes.last() != Some(&b'"') {
        return value.to_owned();
    }
    let inner = value.get(1..value.len() - 1).unwrap_or("");
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        // oracle: a backslash right before the closing quote stays as it is
        if c != '\\' || chars.peek().is_none() {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(e) => out.push(e),
            None => {}
        }
    }
    out
}

/// MOD_UPDATES 1.2 ordering, the way `sscanf("%u.%u.%u.%u")` reads a version: missing parts and
/// everything after the first part that is not a number count as 0.
#[must_use]
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    fn parts(v: &str) -> [u64; 4] {
        let mut out = [0u64; 4];
        let mut rest = v.trim_start();
        for slot in &mut out {
            let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
            if digits == 0 {
                break;
            }
            *slot = rest.get(..digits).and_then(|d| d.parse::<u64>().ok()).unwrap_or(u64::from(u32::MAX)).min(u64::from(u32::MAX));
            rest = rest.get(digits..).unwrap_or("");
            match rest.strip_prefix('.') {
                Some(next) => rest = next,
                None => break,
            }
        }
        out
    }
    parts(a).cmp(&parts(b))
}

// [mods] and [withdrawn] share one line format: github, key, version, date
fn index_key(index: &str, sec: &str, id: &str) -> Option<String> {
    let (_, line) = section(index, sec).into_iter().find(|(k, _)| *k == id)?;
    Some(field(line, 1))
}

/// The index's `[mods]` line for the module: `Some(key text)` when it lists the id.
#[must_use]
pub fn listed_key(index: &str, id: &str) -> Option<String> {
    index_key(index, "mods", id)
}

/// The index's `[withdrawn]` line: a module its author withdrew, whose id stays bound to the key.
#[must_use]
pub fn withdrawn_key(index: &str, id: &str) -> Option<String> {
    index_key(index, "withdrawn", id)
}

/// The n-th comma-separated field of an index line, trimmed of spaces; empty past the last.
#[must_use]
pub fn field(value: &str, n: usize) -> String {
    value.split(',').nth(n).map(|f| f.trim_matches(' ').to_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn sections_and_values() {
        let text = "; head\n[catalog]\nversion = 3\n[mods]\nod = me/od, p256:AAA , 1.0.0, 2026-01-01 ; note\n  other=x/y,p256:B\n[virustotal]\ntrusted = \"a;b\" ; c\n";
        assert_eq!(value(text, "catalog", "version"), Some("3"));
        assert_eq!(value(text, "virustotal", "trusted"), Some("\"a;b\""));
        assert_eq!(listed_key(text, "od").as_deref(), Some("p256:AAA"));
        assert_eq!(listed_key(text, "other").as_deref(), Some("p256:B"));
        assert_eq!(listed_key(text, "none"), None);
        assert_eq!(withdrawn_key(text, "od"), None);
        assert_eq!(field("me/od, p256:AAA , 1.0.0", 0), "me/od");
        assert_eq!(field("a", 3), "");
    }

    #[test]
    fn versions() {
        assert_eq!(compare_versions("1.0.0", "1.0"), Ordering::Equal);
        assert_eq!(compare_versions("0.9.13", "1.0.0"), Ordering::Less);
        assert_eq!(compare_versions("1.10", "1.9"), Ordering::Greater);
        assert_eq!(compare_versions("1.0.0-beta", "1.0.0"), Ordering::Equal);
        assert_eq!(compare_versions("x", "0"), Ordering::Equal);
    }

    #[test]
    fn unquoting() {
        assert_eq!(unquote("\"a\\nb \\\"c\\\"\""), "a\nb \"c\"");
        assert_eq!(unquote("plain"), "plain");
    }
}
