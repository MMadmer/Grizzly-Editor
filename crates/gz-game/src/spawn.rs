//! The gameplay-object roster: every config section that declares `$spawn` and a `class`, as
//! the Spawn tool lists it (`list_spawn_sections`, `UISpawnTool::RefreshList`).

use gz_ltx::Ltx;

/// A section the Spawn tool can place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpawnSection {
    pub section: String,
    /// The list path of its `$spawn` value (`ListPath`).
    pub group: String,
    /// `class` as written.
    pub class: String,
}

/// `UISpawnTool::ListPath`: the `$spawn` value's `\`-separated parts, each trimmed of blanks and
/// a stray leading `;`, empty parts dropped.
#[must_use]
pub fn list_path(spawn: &str) -> String {
    let mut path = String::new();
    for part in spawn.split('\\') {
        let part = part.trim_start_matches([' ', '\t', ';']).trim_end_matches([' ', '\t']);
        if part.is_empty() {
            continue;
        }
        if !path.is_empty() {
            path.push('\\');
        }
        path.push_str(part);
    }
    path
}

/// The roster in the tree's section order.
#[must_use]
pub fn sections(ltx: &Ltx) -> Vec<SpawnSection> {
    ltx.sections()
        .iter()
        .filter(|s| s.get("$spawn").is_some() && s.get("class").is_some())
        .map(|s| SpawnSection {
            section: s.name.to_string(),
            group: list_path(ltx.r_string(&s.name, "$spawn").unwrap_or("")),
            class: ltx.r_string_raw(&s.name, "class").ok().flatten().unwrap_or("").to_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_ltx::{Dialect, parse_file};
    use rustc_hash::FxHashMap;

    #[test]
    fn list_paths() {
        assert_eq!(list_path("weapons\\pistols\\pm"), "weapons\\pistols\\pm");
        assert_eq!(list_path(" ;npc \\  stalkers\t\\\\x "), "npc\\stalkers\\x");
        assert_eq!(list_path(""), "");
    }

    #[test]
    fn roster() {
        let text = "[wpn_pm]\n$spawn = \"weapons\\pistols\\pm\"\nclass = WP_PM\n[no_class]\n$spawn = a\n[no_spawn]\nclass = X\n[child]:wpn_pm\n";
        let mut files = FxHashMap::default();
        files.insert("system.ltx".to_owned(), parse_file(text, Dialect::Editor));
        let ltx = Ltx::assemble_map("system.ltx", &files, Dialect::Editor).unwrap().ltx;
        let got = sections(&ltx);
        let names: Vec<(&str, &str, &str)> =
            got.iter().map(|s| (s.section.as_str(), s.group.as_str(), s.class.as_str())).collect();
        assert_eq!(
            names,
            [("child", "weapons\\pistols\\pm", "WP_PM"), ("wpn_pm", "weapons\\pistols\\pm", "WP_PM")]
        );
    }
}
