//! What is this machine's, not the project's: kept in the editor's home, never in a project.
//!
//! - [`ProjectState`]: `<home>\projects\<project id>.ltx` - where the project was last opened,
//!   the game it is linked to when it is not the default one, the last export target, the recent
//!   scenes, the scene a load was under way for, whether `AGENTS.md` was written. The open tabs
//!   are `<home>\projects\<project id>.tabs.ltx` beside it, written whole by the window.
//! - [`MachineFile`]: `<home>\machine.ltx` - the default game (where it is and how it was found),
//!   the game roots seen before, the editor's library folder.
//!
//! Losing either loses conveniences only. Both are LTX edited through [`LtxDoc`], so a file
//! nobody changed is never rewritten.

use std::path::{Path, PathBuf};

use gz_ltx::LtxDoc;

use crate::ini;

/// The folder of the projects' state files under the home.
pub const STATES: &str = "projects";
/// The editor-wide file under the home.
pub const MACHINE_FILE: &str = "machine.ltx";
const RECENT: usize = 25;

/// `<home>\projects\<id>.ltx`.
#[must_use]
pub fn state_path(home: &Path, id: &str) -> PathBuf {
    home.join(STATES).join(format!("{id}.ltx"))
}

/// `<home>\projects\<id>.tabs.ltx`: the tabs a project brings back.
#[must_use]
pub fn tabs_path(home: &Path, id: &str) -> PathBuf {
    home.join(STATES).join(format!("{id}.tabs.ltx"))
}

/// One project's state on this machine.
#[derive(Clone, Debug, Default)]
pub struct ProjectState {
    doc: LtxDoc,
}

fn text_of(doc: &LtxDoc, section: &str, key: &str) -> String {
    ini::get(doc, section, key).unwrap_or_default().to_owned()
}

impl ProjectState {
    #[must_use]
    pub fn parse(text: &str) -> Self {
        Self { doc: LtxDoc::parse(text) }
    }

    #[must_use]
    pub fn to_text(&self) -> String {
        self.doc.to_string()
    }

    fn with(&self, section: &str, key: &str, value: &str) -> Self {
        let mut doc = self.doc.clone();
        if ini::get(&doc, section, key).unwrap_or_default() != value {
            ini::set(&mut doc, section, key, value);
        }
        Self { doc }
    }

    /// The folder the project was last opened from.
    #[must_use]
    pub fn root(&self) -> String {
        text_of(&self.doc, "project", "root")
    }

    #[must_use]
    pub fn with_root(&self, root: &str) -> Self {
        self.with("project", "root", root)
    }

    /// The project's own game; empty: the default one.
    #[must_use]
    pub fn game_root(&self) -> String {
        text_of(&self.doc, "game", "root").trim_end_matches(['\\', '/']).to_owned()
    }

    #[must_use]
    pub fn with_game_root(&self, root: &str) -> Self {
        self.with("game", "root", root)
    }

    /// Where Build Into Game wrote last.
    #[must_use]
    pub fn export_target(&self) -> String {
        text_of(&self.doc, "build", "export_target")
    }

    #[must_use]
    pub fn with_export_target(&self, target: &str) -> Self {
        self.with("build", "export_target", target)
    }

    /// The scene a load was started for and did not end: the last run stopped in it.
    #[must_use]
    pub fn loading_scene(&self) -> String {
        text_of(&self.doc, "scene", "loading")
    }

    #[must_use]
    pub fn with_loading_scene(&self, rel: &str) -> Self {
        self.with("scene", "loading", rel)
    }

    /// The scenes opened last, newest first, project-relative.
    #[must_use]
    pub fn recent_scenes(&self) -> Vec<String> {
        (0..RECENT)
            .filter_map(|i| ini::get(&self.doc, "recent", &format!("s{i}")).filter(|v| !v.is_empty()))
            .map(str::to_owned)
            .collect()
    }

    #[must_use]
    pub fn with_recent_scene(&self, rel: &str) -> Self {
        let mut list = vec![rel.to_owned()];
        list.extend(self.recent_scenes().into_iter().filter(|s| !s.eq_ignore_ascii_case(rel)));
        list.truncate(RECENT);
        let mut doc = self.doc.clone();
        for i in 0..RECENT {
            ini::set(&mut doc, "recent", &format!("s{i}"), list.get(i).map_or("", String::as_str));
        }
        Self { doc }
    }

    /// `AGENTS.md` was written (or found) once: it is never written again by itself.
    #[must_use]
    pub fn agents_guide_written(&self) -> bool {
        text_of(&self.doc, "project", "agents_guide") == "written"
    }

    #[must_use]
    pub fn with_agents_guide_written(&self) -> Self {
        self.with("project", "agents_guide", "written")
    }
}

/// How the default game was found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Found {
    /// Not yet.
    #[default]
    None,
    /// The author picked it (Preferences > Game, Link Game for every project).
    Preference,
    /// A game this home was linked to before.
    Linked,
    /// The installed Dead Air: Refined, by its installer's entry.
    Installed,
    /// An `xrEngine.exe` next to or above the editor's folder.
    BesideEditor,
}

impl Found {
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Found::None => "",
            Found::Preference => "preference",
            Found::Linked => "linked",
            Found::Installed => "installed",
            Found::BesideEditor => "beside_editor",
        }
    }

    #[must_use]
    pub fn parse(key: &str) -> Self {
        [Found::Preference, Found::Linked, Found::Installed, Found::BesideEditor]
            .into_iter()
            .find(|f| f.key() == key)
            .unwrap_or_default()
    }

    /// The words the author reads after the folder.
    #[must_use]
    pub fn words(self) -> &'static str {
        match self {
            Found::None => "not found",
            Found::Preference => "picked by you",
            Found::Linked => "linked before",
            Found::Installed => "the installed game",
            Found::BesideEditor => "found beside the editor",
        }
    }
}

const SEEN: usize = 16;

/// `<home>\machine.ltx`: the default game and the library.
#[derive(Clone, Debug, Default)]
pub struct MachineFile {
    doc: LtxDoc,
}

impl MachineFile {
    #[must_use]
    pub fn parse(text: &str) -> Self {
        Self { doc: LtxDoc::parse(text) }
    }

    #[must_use]
    pub fn to_text(&self) -> String {
        self.doc.to_string()
    }

    fn with(&self, section: &str, key: &str, value: &str) -> Self {
        let mut doc = self.doc.clone();
        if ini::get(&doc, section, key).unwrap_or_default() != value {
            ini::set(&mut doc, section, key, value);
        }
        Self { doc }
    }

    /// The default game's folder; empty when none is known.
    #[must_use]
    pub fn game_root(&self) -> String {
        text_of(&self.doc, "game", "root").trim_end_matches(['\\', '/']).to_owned()
    }

    #[must_use]
    pub fn found(&self) -> Found {
        Found::parse(&text_of(&self.doc, "game", "found"))
    }

    #[must_use]
    pub fn with_game(&self, root: &str, found: Found) -> Self {
        let mut next = self.with("game", "root", root).with("game", "found", found.key());
        if !root.is_empty() {
            next = next.with_seen(root);
        }
        next
    }

    /// The game roots this home has known, newest first.
    #[must_use]
    pub fn seen(&self) -> Vec<String> {
        (0..SEEN)
            .filter_map(|i| ini::get(&self.doc, "seen", &format!("g{i}")).filter(|v| !v.is_empty()))
            .map(str::to_owned)
            .collect()
    }

    #[must_use]
    pub fn with_seen(&self, root: &str) -> Self {
        let root = root.trim_end_matches(['\\', '/']);
        if root.is_empty() || self.seen().first().is_some_and(|s| crate::paths::same_path(s, root)) {
            return self.clone();
        }
        let mut list = vec![root.to_owned()];
        list.extend(self.seen().into_iter().filter(|s| !crate::paths::same_path(s, root)));
        list.truncate(SEEN);
        let mut doc = self.doc.clone();
        for i in 0..SEEN {
            ini::set(&mut doc, "seen", &format!("g{i}"), list.get(i).map_or("", String::as_str));
        }
        Self { doc }
    }

    /// The editor's library folder (it holds `rawdata` and `gamedata`); empty: beside the editor.
    #[must_use]
    pub fn library_root(&self) -> String {
        text_of(&self.doc, "library", "root").trim_end_matches(['\\', '/']).to_owned()
    }

    #[must_use]
    pub fn with_library_root(&self, root: &str) -> Self {
        self.with("library", "root", root)
    }

    /// Whether the editor registers `.grizzly` for this user (Preferences > General); on unless
    /// turned off.
    #[must_use]
    pub fn associate(&self) -> bool {
        text_of(&self.doc, "association", "on") != "false"
    }

    #[must_use]
    pub fn with_associate(&self, on: bool) -> Self {
        self.with("association", "on", if on { "true" } else { "false" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_projects_state_round_trips() {
        let s = ProjectState::default()
            .with_root("D:\\Mods\\A")
            .with_game_root("D:\\Games\\DA")
            .with_export_target("D:\\Out")
            .with_recent_scene("levels/a.scene")
            .with_recent_scene("levels/b.scene")
            .with_recent_scene("levels/a.scene")
            .with_agents_guide_written();
        let again = ProjectState::parse(&s.to_text());
        assert_eq!(again.root(), "D:\\Mods\\A");
        assert_eq!(again.game_root(), "D:\\Games\\DA");
        assert_eq!(again.export_target(), "D:\\Out");
        assert_eq!(again.recent_scenes(), ["levels/a.scene", "levels/b.scene"]);
        assert!(again.agents_guide_written());
        // the same value again is no change
        assert_eq!(again.with_game_root("D:\\Games\\DA").to_text(), again.to_text());
        assert_eq!(state_path(Path::new("H"), "x"), Path::new("H").join("projects").join("x.ltx"));
    }

    #[test]
    fn the_machine_file_keeps_the_default_game_and_what_was_seen() {
        let m = MachineFile::default()
            .with_game("D:\\Games\\DA\\", Found::Installed)
            .with_seen("E:\\Other")
            .with_library_root("E:\\Library");
        let again = MachineFile::parse(&m.to_text());
        assert_eq!(again.game_root(), "D:\\Games\\DA");
        assert_eq!(again.found(), Found::Installed);
        assert_eq!(again.seen(), ["E:\\Other", "D:\\Games\\DA"]);
        assert_eq!(again.library_root(), "E:\\Library");
        assert!(again.associate());
        assert!(!again.with_associate(false).associate());
    }
}
