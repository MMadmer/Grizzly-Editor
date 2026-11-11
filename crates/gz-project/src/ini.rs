//! Writing the two files the C++ editor kept with `WritePrivateProfileString`: `key=value`
//! lines, no spaces, `\r\n`. An existing line keeps its own spelling ([`LtxDoc::set`]); only a
//! file that has no entry yet to take the shape from is given the Win32 one.

use gz_ltx::LtxDoc;

pub(crate) fn set(doc: &mut LtxDoc, section: &str, key: &str, value: &str) {
    let has_entries =
        doc.sections().any(|s| doc.entries(s).next().is_some()) || doc.entries("").next().is_some();
    if has_entries || doc.has_section(section) {
        doc.set(section, key, value);
        return;
    }
    let mut text = doc.to_string();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push_str("\r\n");
    }
    text.push_str(&format!("[{section}]\r\n{key}={value}\r\n"));
    *doc = LtxDoc::parse(&text);
}

/// A value as `GetPrivateProfileString` hands it back: the outer quotes off.
pub(crate) fn get<'a>(doc: &'a LtxDoc, section: &str, key: &str) -> Option<&'a str> {
    doc.get(section, key).map(gz_ltx::unquote)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_file_takes_the_win32_shape() {
        let mut doc = LtxDoc::new();
        set(&mut doc, "recent", "p0", "D:\\A");
        set(&mut doc, "recent", "p1", "");
        set(&mut doc, "browser", "sort", "name");
        assert_eq!(doc.to_string(), "[recent]\r\np0=D:\\A\r\np1=\r\n\r\n[browser]\r\nsort=name\r\n");
    }

    #[test]
    fn an_existing_file_keeps_its_spelling() {
        let mut doc = LtxDoc::parse("[recent]\r\np0 = D:\\A\r\n");
        set(&mut doc, "recent", "p0", "D:\\B");
        assert_eq!(doc.to_string(), "[recent]\r\np0 = D:\\B\r\n");
        assert_eq!(get(&doc, "recent", "p0"), Some("D:\\B"));
    }
}
