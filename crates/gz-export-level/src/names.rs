//! Names: the level an export is for, op names, path names, a section taken out of a text.

/// `ResolveLevelName`: an explicit `level` wins, otherwise the open scene's file name without
/// its folder and extension (`...\l01_escape.level` -> `l01_escape`). `None` without either.
#[must_use]
pub fn resolve_level(arg: Option<&str>, scene_file_stem: &str) -> Option<String> {
    if let Some(a) = arg.filter(|a| !a.is_empty()) {
        return Some(cut_bytes(a, 127).to_owned());
    }
    let name = cut_bytes(scene_file_stem, 127);
    (!name.is_empty()).then(|| name.to_owned())
}

/// The longest prefix of `s` of at most `n` bytes that ends on a character boundary.
fn cut_bytes(s: &str, n: usize) -> &str {
    let mut cut = s.len().min(n);
    while !s.is_char_boundary(cut) {
        cut -= 1;
    }
    s.get(..cut).unwrap_or_default()
}

/// `SanitizeOpName`: an op name is a section name and a file-name part, so every byte that is
/// not `a-z A-Z 0-9 _ - .` becomes `_` (a Cyrillic letter is two of them); 127 bytes at most;
/// an empty name is `unnamed`.
#[must_use]
pub fn sanitize_op_name(src: &str) -> String {
    let mut out = String::with_capacity(src.len().min(127));
    for b in src.bytes().take(127) {
        let ok = b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.');
        out.push(if ok { char::from(b) } else { '_' });
    }
    if out.is_empty() {
        out.push_str("unnamed");
    }
    out
}

/// `ValidPathName`: a way path's name is a patrol name, an ltx section and a picker entry at
/// once.
#[must_use]
pub fn valid_path_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// `StripIniSection`: the section `name` (any case), header to the next header, taken out of
/// `text`; `true` when there was one.
pub fn strip_ini_section(text: &mut String, name: &str) -> bool {
    let mut out = String::with_capacity(text.len());
    let (mut dropping, mut found) = (false, false);
    let mut pos = 0usize;
    while pos < text.len() {
        let eol = text[pos..].find('\n').map_or(text.len(), |i| pos + i);
        let end = if eol < text.len() { eol + 1 } else { eol };
        let line = &text[pos..end];
        pos = eol + 1;
        let trimmed = line.trim_start_matches([' ', '\t']);
        if trimmed.starts_with('[') {
            let header = trimmed.find(']').map_or("", |close| &trimmed[1..close]);
            dropping = header.eq_ignore_ascii_case(name);
            found |= dropping;
        }
        if !dropping {
            out.push_str(line);
        }
    }
    *text = out;
    found
}

/// A path as the MCP answers name it: forward slashes.
#[must_use]
pub fn json_path(p: &std::path::Path) -> String {
    p.display().to_string().replace('\\', "/")
}

/// The visual a reference names, as configs name visuals: meshes-relative, backslashes, no
/// extension (`meshes/props/box.ogf` -> `props\box`). The last dot anywhere ends it, as
/// `strrchr` does.
#[must_use]
pub fn config_visual(reference: &str) -> String {
    let head = reference.get(..7).unwrap_or_default();
    let src = if head.eq_ignore_ascii_case("meshes\\") || head.eq_ignore_ascii_case("meshes/") {
        reference.get(7..).unwrap_or_default()
    } else {
        reference
    };
    let mut v = cut_bytes(src, 255).replace('/', "\\");
    if let Some(dot) = v.rfind('.') {
        v.truncate(dot);
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_ops_and_paths() {
        assert_eq!(resolve_level(None, "l01_escape").as_deref(), Some("l01_escape"));
        assert_eq!(resolve_level(Some("jupiter"), "l01_escape").as_deref(), Some("jupiter"));
        assert_eq!(resolve_level(Some(""), ""), None);
        assert_eq!(sanitize_op_name("box 1"), "box_1");
        assert_eq!(sanitize_op_name("ящик"), "________");
        assert_eq!(sanitize_op_name(""), "unnamed");
        assert!(valid_path_name("guard_walk-1.a"));
        assert!(!valid_path_name("guard walk"));
        assert_eq!(config_visual("meshes/props/box.ogf"), "props\\box");
        assert_eq!(config_visual("dynamics\\box"), "dynamics\\box");
    }

    #[test]
    fn a_section_is_taken_out() {
        let mut t = "[logic]\r\nactive = walker\r\n[XMS_Order]\r\norder = guard\r\n[walker]\r\npath = x\r\n".to_owned();
        assert!(strip_ini_section(&mut t, "xms_order"));
        assert_eq!(t, "[logic]\r\nactive = walker\r\n[walker]\r\npath = x\r\n");
        assert!(!strip_ini_section(&mut t, "xms_order"));
    }
}
