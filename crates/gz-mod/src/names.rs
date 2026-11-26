//! What a module calls its own. A section, a string id, a dialog id, a story id, an info portion,
//! a level of its own, a mode, a console command and a new file under `gamedata` are one name for
//! the whole game: two modules that pick one collide, and the game keeps one. A name the module's
//! id is a word of is the module's own - another module's id is another word.
//!
//! The id is a whole word of the name: bounded by the name's start or end or by `_`, `.` or `-`
//! (`od_rifle`, `wpn_od_ak74`, `st_od_items.xml` are module `od`'s; `odd_rifle` is not), so the
//! game's own prefixes (`wpn_`, `ammo_`, `st_`), which its scripts classify sections by, stay where
//! they are. Case does not matter: the game reads these names without it.

/// Whether a character ends a word of a name.
fn bound(c: Option<u8>) -> bool {
    c.is_none_or(|c| matches!(c, b'_' | b'.' | b'-'))
}

/// Whether `name` is named after `module`: the module's id is a whole word of it. An empty module
/// id names nothing; a name that is the id alone is the module's.
#[must_use]
pub fn is_own(name: &str, module: &str) -> bool {
    let (n, m) = (name.to_ascii_lowercase(), module.to_ascii_lowercase());
    if m.is_empty() || n.is_empty() {
        return false;
    }
    let (nb, mb) = (n.as_bytes(), m.as_bytes());
    let mut from = 0;
    while let Some(at) = n.get(from..).and_then(|rest| rest.find(m.as_str())).map(|i| i + from) {
        let before = at.checked_sub(1).and_then(|i| nb.get(i).copied());
        let after = nb.get(at + mb.len()).copied();
        if bound(before) && bound(after) {
            return true;
        }
        from = at + 1;
    }
    false
}

/// Whether a path under the module's game data is named after the module: one of its folders or
/// the file is (`textures/od/rifle.dds`, `scripts/od_main.script`, `configs/text/eng/st_od.xml`).
#[must_use]
pub fn is_own_path(path: &str, module: &str) -> bool {
    path.split(['/', '\\']).any(|part| is_own(part, module))
}

/// The name a module's own would have: the module's id, then the name.
#[must_use]
pub fn own_form(name: &str, module: &str) -> String {
    format!("{module}_{name}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_module_id_is_a_whole_word_of_the_name() {
        for name in ["od_rifle", "wpn_od_ak74", "od", "OD.Rifle", "st_od", "a-od-b", "x_od"] {
            assert!(is_own(name, "od"), "{name}");
        }
        for name in ["odd_rifle", "wpn_odak", "rifle", "", "pod_x", "x_odx"] {
            assert!(!is_own(name, "od"), "{name}");
        }
        assert!(!is_own("od_rifle", ""));
        // a module id that holds the bounds itself
        assert!(is_own("wpn_my.mod_x", "my.mod") && !is_own("wpn_my.modx", "my.mod"));
        // the first place the id stands is not a word, a later one is
        assert!(is_own("odx_od", "od"));
    }

    #[test]
    fn a_path_is_the_modules_by_a_folder_or_the_file() {
        assert!(is_own_path("textures/od/rifle.dds", "od"));
        assert!(is_own_path("scripts\\od_main.script", "od"));
        assert!(is_own_path("configs/text/eng/st_od.xml", "od"));
        assert!(!is_own_path("textures/weapons/rifle.dds", "od"));
        assert_eq!(own_form("rifle", "od"), "od_rifle");
    }
}
