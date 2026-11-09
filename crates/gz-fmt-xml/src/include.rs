//! `ParseFile` of `xrXMLParser.cpp`: the textual pre-pass that expands `#include` lines.

use crate::doc::XmlWarning;

const MAX_DEPTH: usize = 32;

fn is_term(c: u8) -> bool {
    c == b'\r' || c == b'\n'
}

/// The lines as `IReader::r_string` returns them: a run of line breaks ends a line, so blank
/// lines vanish.
fn lines(text: &str) -> impl Iterator<Item = &str> {
    let b = text.as_bytes();
    let mut pos = 0;
    std::iter::from_fn(move || {
        if pos >= b.len() {
            return None;
        }
        let start = pos;
        let mut sz = 0;
        while pos < b.len() {
            pos += 1;
            sz += 1;
            if b.get(pos).copied().is_some_and(is_term) {
                while b.get(pos).copied().is_some_and(is_term) {
                    pos += 1;
                }
                break;
            }
        }
        let line = text.get(start..start + sz).unwrap_or("");
        Some(line.split('\0').next().unwrap_or(""))
    })
}

/// Expands a file the way `CXml::Load` does before it parses: every line written back with
/// `\r\n`, a line that starts with `#` and holds `#include` replaced by the file named between
/// its first pair of quotes. `include` gets the name as written (relative to the config root,
/// as the game resolves it) and returns that file's text.
///
/// The C++ stops on a missing include; here it is a warning and the line is dropped.
pub fn preprocess(text: &str, include: &dyn Fn(&str) -> Option<String>) -> (String, Vec<XmlWarning>) {
    let mut out = String::with_capacity(text.len() + 64);
    let mut warnings = Vec::new();
    expand(text, include, &mut out, &mut warnings, 0, &mut Vec::new());
    (out, warnings)
}

fn expand(
    text: &str,
    include: &dyn Fn(&str) -> Option<String>,
    out: &mut String,
    warnings: &mut Vec<XmlWarning>,
    depth: usize,
    chain: &mut Vec<String>,
) {
    for (n, line) in lines(text).enumerate() {
        if line.starts_with('#') && line.contains("#include") {
            let name = line.split('"').nth(1).unwrap_or("").trim_matches(|c: char| u32::from(c) <= 0x20).to_owned();
            let lineno = u32::try_from(n + 1).unwrap_or(u32::MAX);
            if depth >= MAX_DEPTH || chain.iter().any(|c| c.eq_ignore_ascii_case(&name)) {
                warnings.push(XmlWarning { line: lineno, message: format!("The include of '{name}' loops back on itself; it is skipped.") });
                continue;
            }
            match include(&name) {
                Some(body) => {
                    chain.push(name);
                    expand(&body, include, out, warnings, depth + 1, chain);
                    chain.pop();
                }
                None => warnings.push(XmlWarning { line: lineno, message: format!("Can't find include file '{name}'; it is skipped.") }),
            }
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_includes_and_drops_blank_lines() {
        let inc = |name: &str| -> Option<String> {
            match name {
                "ui\\a.xml" => Some("<a/>\n\n#include \"ui\\b.xml\"\n".into()),
                "ui\\b.xml" => Some("<b/>".into()),
                _ => None,
            }
        };
        let (out, w) = preprocess("<w>\r\n\r\n#include \"ui\\a.xml\"\r\n #include \"x\"\r\n#include \"gone.xml\"\r\n</w>", &inc);
        assert_eq!(out, "<w>\r\n<a/>\r\n<b/>\r\n #include \"x\"\r\n</w>\r\n");
        assert_eq!(w.len(), 1);
        let (_, w) = preprocess("#include \"self.xml\"", &|_| Some("#include \"self.xml\"".into()));
        assert_eq!(w.len(), 1);
    }
}
