//! Text rules of `mod.ltx` as the C++ `Load` / `Save` and the game apply them
//! (`MOD_UPDATES.md` 1.1: quoting).

/// What the C++ `Trim` takes off both ends.
pub const WS: &[char] = gz_ltx::doc::MANIFEST_WS;

#[must_use]
pub fn trim(s: &str) -> &str {
    s.trim_matches(WS)
}

/// Where the `;` that starts a comment is, `None` when the line has none: `gz_ltx`'s manifest
/// rule (a `;` inside a quoted value belongs to it, `//` is no comment: a website is
/// `https://...`).
pub use gz_ltx::doc::find_manifest_comment as find_comment;

/// `"..."` to text: `\n`, `\"` and `\\` are decoded, any other backslash stays as written.
/// `None` when `val` is not exactly one quoted string.
#[must_use]
pub fn unquote(val: &str) -> Option<String> {
    let b = val.as_bytes();
    if b.len() < 2 || b.first() != Some(&b'"') {
        return None;
    }
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 1;
    while let Some(&c) = b.get(i) {
        if c == b'"' {
            break;
        }
        if c == b'\\'
            && let Some(&next) = b.get(i + 1)
            && matches!(next, b'n' | b'"' | b'\\')
        {
            out.push(if next == b'n' { b'\n' } else { next });
            i += 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    // never closed, or text after the closing quote
    if i + 1 != b.len() {
        return None;
    }
    String::from_utf8(out).ok()
}

/// A display text written quoted: `\`, `"` and line breaks escaped.
#[must_use]
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `name` and `author` stay bare unless the bare form would not read back as written.
#[must_use]
pub fn needs_quotes(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    if s.contains([';', '"', '\n']) {
        return true;
    }
    s.starts_with([' ', '\t']) || s.ends_with([' ', '\t'])
}

/// A display text the way `AppendText` writes it: carriage returns dropped (CRLF from a paste
/// becomes one `\n`), quoted when `always` or when the bare form would not survive.
#[must_use]
pub fn display_value(value: &str, always: bool) -> String {
    let text: String = value.chars().filter(|&c| c != '\r').collect();
    if always || needs_quotes(&text) { quote(&text) } else { text }
}

/// A value that may be quoted (`name`, `author`, `description`, `help`, `args`): its text.
#[must_use]
pub fn text_value(value: &str) -> String {
    unquote(value).unwrap_or_else(|| value.to_owned())
}

/// The C++ `SplitList`: entries trimmed, empty ones dropped; `\r` counts as `\n` for a line
/// list.
#[must_use]
pub fn split_list(src: &str, sep: char) -> Vec<String> {
    let parts: Vec<&str> =
        if sep == '\n' { src.split(['\n', '\r']).collect() } else { src.split(sep).collect() };
    parts.into_iter().map(trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

/// The characters of `text` the game's cp1251 cannot show, as `NqUtil::Cp1251Covers` names
/// them: the first six distinct ones, space separated. `None` when every character fits.
#[must_use]
pub fn outside_cp1251(text: &str) -> Option<String> {
    if text.is_ascii() {
        return None;
    }
    let mut seen: Vec<char> = Vec::new();
    let mut bad = String::new();
    let mut buf = [0u8; 4];
    for c in text.chars() {
        if c.is_ascii() || gz_base::text::fits_cp1251(c.encode_utf8(&mut buf)) || seen.contains(&c) {
            continue;
        }
        seen.push(c);
        if seen.len() > 6 {
            break;
        }
        if !bad.is_empty() {
            bad.push(' ');
        }
        bad.push(c);
    }
    if seen.is_empty() { None } else { Some(bad) }
}

/// `1 file`, `3 files`.
#[must_use]
pub fn count_text(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_respect_quotes() {
        assert_eq!(find_comment("name = x ; c"), Some(9));
        assert_eq!(find_comment("name = \"a; b\" ; c"), Some(14));
        assert_eq!(find_comment("d = \"say \\\"hi; there\\\"\""), None);
        assert_eq!(find_comment("website = https://www.moddb.com/mods/x"), None);
        // an unclosed quote is an ordinary character
        assert_eq!(find_comment("x = \"open ; c"), Some(10));
    }

    #[test]
    fn quoting_round_trips() {
        for s in ["plain", "a; b", "say \"hi\"", "two\nlines", "back\\slash", ""] {
            assert_eq!(unquote(&quote(s)).as_deref(), Some(s));
        }
        assert_eq!(unquote("\"x\" tail"), None);
        assert_eq!(unquote("\"open"), None);
        assert_eq!(unquote("bare"), None);
        assert_eq!(unquote("\"keep \\t\""), Some("keep \\t".to_owned()));
        assert!(needs_quotes(" lead"));
        assert!(!needs_quotes("Overnight Delivery"));
        assert_eq!(display_value("a\r\nb", true), "\"a\\nb\"");
        assert_eq!(display_value("Madmer", false), "Madmer");
    }

    #[test]
    fn lists_and_cp1251() {
        assert_eq!(split_list(" a, ,b ,", ','), ["a", "b"]);
        assert_eq!(split_list("docs\r\n*.psd\n\nwip", '\n'), ["docs", "*.psd", "wip"]);
        assert_eq!(outside_cp1251("Привет"), None);
        assert_eq!(outside_cp1251("a★b→★").as_deref(), Some("★ →"));
        assert_eq!(count_text(1, "file", "files"), "1 file");
    }
}
