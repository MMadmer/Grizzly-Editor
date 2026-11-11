//! The metadata rules - the game enforces the same ones (`MOD_UPDATES.md`, section 1) - and the
//! manifest's own check before a build.

use gz_base::Problem;

use crate::model::Manifest;
use crate::text::{outside_cp1251, trim};

fn id_char(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'-')
}

/// The XMS id charset: `[a-z0-9_.-]+`.
#[must_use]
pub fn validate_id(id: &str) -> bool {
    !id.is_empty() && id.bytes().all(id_char)
}

/// A mode id follows the module id charset, but empty is allowed (= all modes).
#[must_use]
pub fn validate_mode_id(id: &str) -> bool {
    id.is_empty() || validate_id(id)
}

/// `min_parts`..`max_parts` dot-separated decimal numbers, 9 digits each at most.
#[must_use]
pub fn dotted_number(v: &str, min_parts: usize, max_parts: usize) -> bool {
    if v.is_empty() {
        return false;
    }
    let (mut parts, mut digits) = (1usize, 0usize);
    for c in v.bytes() {
        if c.is_ascii_digit() {
            digits += 1;
            if digits > 9 {
                return false;
            }
        } else if c == b'.' {
            parts += 1;
            if digits == 0 || parts > max_parts {
                return false;
            }
            digits = 0;
        } else {
            return false;
        }
    }
    digits > 0 && parts >= min_parts
}

/// `MOD_UPDATES.md` 1.2: 1-4 dot-separated numbers and nothing else, 32 characters in all - a
/// release has to be orderable.
#[must_use]
pub fn validate_version(version: &str) -> bool {
    dotted_number(version, 1, 4) && version.len() <= 32
}

/// `MOD_UPDATES.md` 1.3: https, the host exactly `(www.)ap-pro.ru` or `(www.)moddb.com`, no
/// user info or port, a limited charset, 512 bytes at most; empty is valid.
#[must_use]
pub fn validate_website(url: &str) -> bool {
    if url.is_empty() {
        return true;
    }
    if url.len() > 512 {
        return false;
    }
    let Some(rest) = url.strip_prefix("https://") else { return false };
    // the host is everything up to the first '/', compared whole: "user@host", "host:port"
    // and "host.evil.com" all land inside it and fail the match
    let (host, path) = match rest.find('/') {
        Some(i) => (rest.get(..i).unwrap_or(""), rest.get(i..).unwrap_or("")),
        None => (rest, ""),
    };
    const HOSTS: [&str; 4] = ["ap-pro.ru", "www.ap-pro.ru", "moddb.com", "www.moddb.com"];
    if !HOSTS.iter().any(|h| h.eq_ignore_ascii_case(host)) {
        return false;
    }
    path.bytes().all(|c| c.is_ascii_alphanumeric() || b"-._~/?#&=%+".contains(&c))
}

/// `MOD_UPDATES.md` 1.4: `owner/repo`; empty is valid.
#[must_use]
pub fn validate_github_repo(repo: &str) -> bool {
    if repo.is_empty() {
        return true;
    }
    let Some((owner, name)) = repo.split_once('/') else { return false };
    if owner.is_empty() || owner.len() > 39 || name.is_empty() || name.len() > 100 {
        return false;
    }
    if !owner.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
        return false;
    }
    // a second '/' is not in the repository charset, so "a/b/c" fails here
    if !name.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_')) {
        return false;
    }
    name != "." && name != ".."
}

/// A pasted `https://github.com/owner/repo[.git][/...]` reduced to `owner/repo`; anything else
/// comes back trimmed for [`validate_github_repo`] to judge.
#[must_use]
pub fn normalize_github_repo(input: &str) -> String {
    let mut out = trim(input).to_owned();
    const PREFIXES: [&str; 6] = [
        "https://github.com/",
        "http://github.com/",
        "https://www.github.com/",
        "http://www.github.com/",
        "github.com/",
        "www.github.com/",
    ];
    for p in PREFIXES {
        let hit = out.get(..p.len()).is_some_and(|h| h.eq_ignore_ascii_case(p));
        if !hit {
            continue;
        }
        out.drain(..p.len());
        // a page URL carries more than the repository: ?tab=, #readme, /tree/main
        if let Some(q) = out.find(['?', '#']) {
            out.truncate(q);
        }
        if let Some(owner_end) = out.find('/')
            && let Some(repo_end) = out.get(owner_end + 1..).and_then(|r| r.find('/'))
        {
            out.truncate(owner_end + 1 + repo_end);
        }
        break;
    }
    // clone URLs end in .git; GitHub allows no repository name that does
    if out.len() > 4 && out.get(out.len() - 4..).is_some_and(|t| t.eq_ignore_ascii_case(".git")) {
        out.truncate(out.len() - 4);
    }
    out
}

/// Letters, digits and `_`: what the runtime reads as a global's name.
#[must_use]
pub fn valid_global_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
}

/// The manifest's own check before a build, in the C++ order and with its messages: the id,
/// the targeting choice, the declared modes; the warnings about the mode titles and about a
/// module that turns the game's updates off. Console commands are [`crate::validate_commands`]:
/// they need the project's scripts.
#[must_use]
pub fn validate(m: &Manifest) -> Vec<Problem> {
    let mut out = Vec::new();
    if !validate_id(&m.id) {
        out.push(
            Problem::error("mod.id", "module id missing or invalid in mod.ltx (allowed: a-z 0-9 _ . -)")
                .in_field("id"),
        );
    }
    // A finished module always states which game it is for. "Nothing chosen" is not a third
    // option - it is how content ends up applied in a campaign it was never built for.
    if m.mode.is_empty() && m.provides_modes.is_empty() && !m.target.eq_ignore_ascii_case("default") {
        out.push(
            Problem::error(
                "mod.target",
                "no target game mode: open Mod > Mod Settings, Game Mode, and pick one (the ordinary game, a mode of the linked game, or a new mode this module adds)",
            )
            .in_field("target"),
        );
    }
    if m.disable_game_updates {
        out.push(
            Problem::warning(
                "mod.updates_off",
                "This module turns off the game's updates and bug reports for everyone who has it.",
            )
            .in_field("disable_game_updates"),
        );
    }
    for mode in m.provides_modes.iter().filter(|p| !p.id.is_empty()) {
        if !validate_mode_id(&mode.id) || mode.id.len() > 64 {
            out.push(
                Problem::error(
                    "mod.mode_id",
                    format!(
                        "invalid declared mode id '{}' (allowed: a-z 0-9 _ . -, up to 64 chars)",
                        mode.id
                    ),
                )
                .about(mode.id.as_str()),
            );
        }
        // the game shows the title in cp1251, and a build driven over MCP never sees the
        // field's own warning
        if let Some(bad) = outside_cp1251(&mode.title) {
            out.push(
                Problem::warning(
                    "mod.mode_title",
                    format!("Mode '{}': the title has characters outside cp1251 ({bad}), the player will see a look-alike or '?'.", mode.id),
                )
                .about(mode.id.as_str()),
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_versions() {
        assert!(validate_id("od.mod-2_x"));
        assert!(!validate_id("Od"));
        assert!(!validate_id(""));
        assert!(validate_mode_id(""));
        assert!(validate_version("1.2.0"));
        assert!(validate_version("1"));
        assert!(!validate_version("1.2.3.4.5"));
        assert!(!validate_version("1..2"));
        assert!(!validate_version("1.2."));
        assert!(!validate_version("1234567890"));
        assert!(!validate_version("v1.0"));
        assert!(dotted_number("1.4.1", 3, 3) && !dotted_number("1.4", 3, 3));
    }

    #[test]
    fn websites() {
        assert!(validate_website(""));
        assert!(validate_website("https://ap-pro.ru/stuff/x/od-r123/"));
        assert!(validate_website("https://WWW.ModDB.com"));
        assert!(!validate_website("http://moddb.com/x"));
        assert!(!validate_website("https://moddb.com.evil.com/x"));
        assert!(!validate_website("https://user@moddb.com/x"));
        assert!(!validate_website("https://moddb.com:443/x"));
        assert!(!validate_website("https://moddb.com/a b"));
    }

    #[test]
    fn repositories() {
        assert!(validate_github_repo("Madmer/agroprom-story"));
        assert!(validate_github_repo(""));
        assert!(!validate_github_repo("a/b/c"));
        assert!(!validate_github_repo("a/.."));
        assert!(!validate_github_repo("noslash"));
        assert_eq!(normalize_github_repo(" https://github.com/Madmer/od/tree/main?tab=x "), "Madmer/od");
        assert_eq!(normalize_github_repo("GitHub.com/Madmer/od.git"), "Madmer/od");
        assert_eq!(normalize_github_repo("https://github.com/Madmer/od#readme"), "Madmer/od");
        assert_eq!(normalize_github_repo("Madmer/od"), "Madmer/od");
        assert!(valid_global_name("kills_2") && !valid_global_name("a-b"));
    }

    #[test]
    fn the_build_check() {
        let mut m = Manifest { id: "Bad Id".into(), ..Manifest::default() };
        let p = validate(&m);
        assert_eq!(p.iter().map(|p| p.code.as_str()).collect::<Vec<_>>(), ["mod.id", "mod.target"]);
        m.id = "od".into();
        m.target = "Default".into();
        assert!(validate(&m).is_empty());
        m.provides_modes.push(crate::ProvidesMode { id: "Bad".into(), title: "Ночь ★".into() });
        m.disable_game_updates = true;
        let codes: Vec<String> = validate(&m).iter().map(|p| p.code.to_string()).collect();
        assert_eq!(codes, ["mod.updates_off", "mod.mode_id", "mod.mode_title"]);
    }
}
