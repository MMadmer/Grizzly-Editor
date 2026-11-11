//! What the build leaves out.
//!
//! A project folder is the author's working place as well as the module. Tooling never ships
//! ([`gz_project::is_tooling_entry`]), nor the editor's scratch and the sources at the root; the
//! rest is the author's call, made in `[exclude]`: a project-relative path (a folder takes
//! everything below it) or a pattern - with no slash a NAME at any depth (`*.psd`), with one a
//! path from the root (`docs/*.md`). `*` and `?` never cross a folder boundary; case and slash
//! direction do not matter.

use gz_project::{is_editor_only_entry, is_source_only_entry, is_tooling_entry};

use crate::text::trim;

/// `/Docs/Drafts/` and `docs\drafts` are one rule: backslashes, no leading `.\` or separator,
/// no trailing one.
#[must_use]
pub fn normalize_rel(input: &str) -> String {
    let r = input.replace('/', "\\");
    let mut r = trim(&r);
    loop {
        if let Some(rest) = r.strip_prefix(".\\") {
            r = rest;
        } else if let Some(rest) = r.strip_prefix('\\') {
            r = rest;
        } else {
            break;
        }
    }
    r.trim_end_matches('\\').to_owned()
}

/// `*` and `?` against `text`, ASCII case ignored; neither crosses `stop` (`\` in a path, `0`
/// for a plain name).
fn glob_match(mask: &[u8], text: &[u8], stop: u8) -> bool {
    let (mut m, mut t) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while let Some(&tc) = text.get(t) {
        if mask.get(m) == Some(&b'*') {
            m += 1;
            star = Some((m, t));
            continue;
        }
        let one = match mask.get(m) {
            Some(b'?') => tc != stop || stop == 0,
            Some(&mc) => mc.eq_ignore_ascii_case(&tc),
            None => false,
        };
        if one {
            m += 1;
            t += 1;
            continue;
        }
        // the star takes one more character - never a folder boundary
        let Some((star_m, star_t)) = star else { return false };
        if stop != 0 && text.get(star_t) == Some(&stop) {
            return false;
        }
        m = star_m;
        t = star_t + 1;
        star = Some((star_m, t));
    }
    while mask.get(m) == Some(&b'*') {
        m += 1;
    }
    m == mask.len()
}

/// The `[exclude]` entry that takes `rel` out - the entry itself or a folder above it - as
/// written in the file.
#[must_use]
pub fn exclude_match<'a>(rules: &'a [String], rel: &str) -> Option<&'a String> {
    let rel = normalize_rel(rel);
    if rel.is_empty() {
        return None;
    }
    let ends: Vec<usize> =
        rel.match_indices('\\').map(|(i, _)| i).chain(std::iter::once(rel.len())).collect();
    for rule in rules {
        let r = normalize_rel(rule);
        if r.is_empty() {
            continue;
        }
        let wild = r.contains(['*', '?']);
        let path = r.contains('\\');
        for &end in &ends {
            let prefix = rel.get(..end).unwrap_or("");
            let hit = if !wild {
                prefix.eq_ignore_ascii_case(&r)
            } else if path {
                glob_match(r.as_bytes(), prefix.as_bytes(), b'\\')
            } else {
                let name = prefix.rsplit('\\').next().unwrap_or(prefix);
                glob_match(r.as_bytes(), name.as_bytes(), 0)
            };
            if hit {
                return Some(rule);
            }
        }
    }
    None
}

/// Whether a project-relative entry is out of the build: tooling at any depth, the editor's
/// scratch and the sources at the root, whatever `[exclude]` names. Every walk that feeds a
/// build asks it - the graphs, the characters, the export.
#[must_use]
pub fn outside_build(rules: &[String], rel: &str) -> bool {
    let rel = normalize_rel(rel);
    if rel.is_empty() {
        return false;
    }
    for (n, seg) in rel.split('\\').enumerate() {
        if is_tooling_entry(seg) {
            return true;
        }
        // the editor's scratch and the sources are root entries only
        if n == 0 && (is_editor_only_entry(seg) || is_source_only_entry(seg)) {
            return true;
        }
    }
    exclude_match(rules, &rel).is_some()
}

/// What the content browser says about an entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShipState {
    Shipped,
    /// Its own `[exclude]` line takes it out.
    Excluded,
    /// Another rule does - a folder above it, a pattern - named here.
    ExcludedByRule(String),
    /// It never ships at all: the editor's scratch, the sources.
    Never,
}

#[must_use]
pub fn ship_state(rules: &[String], rel: &str) -> ShipState {
    let rel = normalize_rel(rel);
    if rel.is_empty() {
        return ShipState::Shipped;
    }
    let first = rel.split('\\').next().unwrap_or("");
    if is_editor_only_entry(first) || is_source_only_entry(first) {
        return ShipState::Never;
    }
    match exclude_match(rules, &rel) {
        None => ShipState::Shipped,
        Some(rule) if normalize_rel(rule).eq_ignore_ascii_case(&rel) => ShipState::Excluded,
        Some(rule) => ShipState::ExcludedByRule(rule.clone()),
    }
}

/// `[exclude]` after the browser's Exclude from Build (`excluded`) or Include in Build of one
/// entry: `Ok(None)` when nothing changes, `Err` with the C++ text when it cannot be done.
pub fn set_excluded(rules: &[String], rel: &str, excluded: bool) -> Result<Option<Vec<String>>, String> {
    let rel = normalize_rel(rel);
    if rel.is_empty() {
        return Err("the project root cannot be excluded - that is the whole module".to_owned());
    }
    if rel.eq_ignore_ascii_case(crate::model::FILE) {
        return Err("mod.ltx always ships - it is what makes the folder a module".to_owned());
    }
    // stored the way people write paths in a text file
    let stored = rel.replace('\\', "/");
    let mut next: Vec<String> = rules.to_vec();
    if excluded {
        if exclude_match(rules, &rel).is_some() {
            return Ok(None);
        }
        // an entry below the new one says nothing any more
        next.retain(|r| {
            let r = normalize_rel(r);
            let below = r.len() > rel.len()
                && r.as_bytes().get(rel.len()) == Some(&b'\\')
                && r.get(..rel.len()).is_some_and(|h| h.eq_ignore_ascii_case(&rel))
                && !r.contains(['*', '?']);
            !below
        });
        next.push(stored);
    } else {
        next.retain(|r| !normalize_rel(r).eq_ignore_ascii_case(&rel));
        if let Some(still) = exclude_match(&next, &rel) {
            return Err(format!("still excluded by '{still}' in mod.ltx [exclude]"));
        }
        if next.len() == rules.len() {
            return Ok(None);
        }
    }
    Ok(Some(next))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn rules(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn patterns() {
        let r = rules(&["docs", "*.psd", "art/raw*/*.tga", "/Design/"]);
        assert_eq!(exclude_match(&r, "docs/notes.md").map(String::as_str), Some("docs"));
        assert_eq!(exclude_match(&r, "DOCS"), Some(&r[0]));
        assert!(exclude_match(&r, "docsx/a").is_none());
        assert_eq!(exclude_match(&r, "art\\deep\\cover.PSD"), Some(&r[1]));
        assert!(exclude_match(&r, "art/raw1/a.tga").is_some());
        assert!(exclude_match(&r, "art/raw1/sub/a.tga").is_none());
        assert!(exclude_match(&r, "design/story.md").is_some());
        assert!(exclude_match(&r, "").is_none());
        // a star never crosses a folder
        assert!(exclude_match(&rules(&["a*b"]), "a/x/b").is_none());
        assert!(exclude_match(&rules(&["a?c"]), "abc").is_some());
        assert_eq!(normalize_rel(" ./\\Docs/Drafts/ "), "Docs\\Drafts");
    }

    #[test]
    fn what_never_ships() {
        let r = rules(&["wip"]);
        assert!(outside_build(&r, ".git/objects/x"));
        assert!(outside_build(&r, "gamedata/textures/Thumbs.db"));
        assert!(outside_build(&r, "temp/x"));
        assert!(outside_build(&r, "rawdata/scene.level"));
        assert!(!outside_build(&r, "gamedata/temp/x"));
        assert!(outside_build(&r, "wip/char.npcasset"));
        assert!(!outside_build(&r, "gamedata/scripts/od.script"));
        assert_eq!(ship_state(&r, "releases/1.0"), ShipState::Never);
        assert_eq!(ship_state(&r, "wip"), ShipState::Excluded);
        assert_eq!(ship_state(&r, "wip/a"), ShipState::ExcludedByRule("wip".into()));
        assert_eq!(ship_state(&r, "gamedata"), ShipState::Shipped);
    }

    #[test]
    fn the_browser_toggle() {
        let r = rules(&["docs", "*.psd", "wip_char"]);
        let next = set_excluded(&r, "design", true).unwrap().unwrap();
        assert_eq!(next, ["docs", "*.psd", "wip_char", "design"]);
        assert_eq!(set_excluded(&next, "design/story.md", true), Ok(None));
        assert_eq!(
            set_excluded(&next, "design/story.md", false),
            Err("still excluded by 'design' in mod.ltx [exclude]".into())
        );
        assert!(set_excluded(&next, "mod.ltx", true).is_err());
        assert!(set_excluded(&next, "/", true).is_err());
        let folded = set_excluded(&rules(&["docs/a", "docs/*.md", "x"]), "docs", true).unwrap().unwrap();
        assert_eq!(folded, ["docs/*.md", "x", "docs"]);
        assert_eq!(set_excluded(&folded, "x", false).unwrap().unwrap(), ["docs/*.md", "docs"]);
        assert_eq!(set_excluded(&folded, "never", false), Ok(None));
    }
}
