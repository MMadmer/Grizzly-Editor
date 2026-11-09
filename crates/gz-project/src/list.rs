//! The project list, `_projects.ini` in the editor's root: `[recent] p0..p11` (the last opened
//! first), `create_parent` (the New Project form's location) and `[browser] sort`.
//!
//! The file is edited through [`LtxDoc`], so a list nobody changed writes back byte for byte
//! (QA restores and compares it). A folder that is gone stays on the list: a drive that is not
//! plugged in is not a project nobody wants any more.

use std::path::Path;

use gz_ltx::LtxDoc;
use serde::Serialize;

use crate::{ini, paths};

pub const FILE: &str = "_projects.ini";
pub const MAX_RECENT: usize = 12;

/// What a listed folder is now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryState {
    /// The folder holds `project.ltx` or `mod.ltx`.
    Project,
    /// The folder does not exist.
    Missing,
    /// A folder with neither in it.
    NotAProject,
}

impl EntryState {
    /// As `list_projects` names it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            EntryState::Project => "project",
            EntryState::Missing => "missing",
            EntryState::NotAProject => "not_a_project",
        }
    }
}

/// A project is a folder with `project.ltx` - or with `mod.ltx` alone (a fresh clone of a
/// mod's repository; opening it writes `project.ltx`).
#[must_use]
pub fn has_manifest(folder: &Path) -> bool {
    std::fs::symlink_metadata(folder.join("project.ltx")).is_ok() || std::fs::symlink_metadata(folder.join("mod.ltx")).is_ok()
}

/// What a listed folder is now. Two or three attribute reads.
#[must_use]
pub fn entry_state(folder: &str) -> EntryState {
    let dir = Path::new(folder);
    if !std::fs::metadata(dir).is_ok_and(|m| m.is_dir()) {
        return EntryState::Missing;
    }
    if has_manifest(dir) { EntryState::Project } else { EntryState::NotAProject }
}

/// How the Start page orders its cards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Recent,
    Name,
}

impl Sort {
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Sort::Recent => "recent",
            Sort::Name => "name",
        }
    }

    /// `recent` or `name`, exactly.
    #[must_use]
    pub fn parse(key: &str) -> Option<Sort> {
        match key {
            "recent" => Some(Sort::Recent),
            "name" => Some(Sort::Name),
            _ => None,
        }
    }
}

/// `_projects.ini`. Cheap to clone.
#[derive(Clone, Debug, Default)]
pub struct ProjectList {
    doc: LtxDoc,
}

impl ProjectList {
    #[must_use]
    pub fn parse(text: &str) -> Self {
        Self { doc: LtxDoc::parse(text) }
    }

    /// The file's text.
    #[must_use]
    pub fn to_text(&self) -> String {
        self.doc.to_string()
    }

    /// The listed folders, the last opened first, spelled as the file has them.
    #[must_use]
    pub fn entries(&self) -> Vec<String> {
        (0..MAX_RECENT)
            .filter_map(|i| ini::get(&self.doc, "recent", &format!("p{i}")).filter(|v| !v.is_empty()).map(str::to_owned))
            .collect()
    }

    /// The New Project form's location, when the file names one.
    #[must_use]
    pub fn create_parent(&self) -> Option<&str> {
        ini::get(&self.doc, "recent", "create_parent").filter(|v| !v.is_empty())
    }

    #[must_use]
    pub fn sort(&self) -> Sort {
        match ini::get(&self.doc, "browser", "sort") {
            Some(v) if v.eq_ignore_ascii_case("name") => Sort::Name,
            _ => Sort::Recent,
        }
    }

    /// The listed spelling of `path`, when the list names it. Only a full path is looked for.
    #[must_use]
    pub fn find_listed(&self, path: &str) -> Option<String> {
        if !paths::is_absolute(path) {
            return None;
        }
        self.entries().into_iter().find(|e| paths::same_path(e, path))
    }

    fn write_entries(doc: &mut LtxDoc, entries: &[String]) {
        for i in 0..MAX_RECENT {
            ini::set(doc, "recent", &format!("p{i}"), entries.get(i).map_or("", String::as_str));
        }
    }

    /// The list after `project` opened: it floats to the top, its duplicates collapse, the rest
    /// keep their order (twelve at most), and the form's location is written along.
    #[must_use]
    pub fn opened(&self, project: &str, create_parent: &str) -> Self {
        let mut list = vec![project.to_owned()];
        list.extend(self.entries().into_iter().filter(|e| !e.eq_ignore_ascii_case(project)));
        list.truncate(MAX_RECENT);
        let mut doc = self.doc.clone();
        Self::write_entries(&mut doc, &list);
        ini::set(&mut doc, "recent", "create_parent", create_parent);
        Self { doc }
    }

    /// The list without `path` (every spelling of it).
    #[must_use]
    pub fn without(&self, path: &str) -> Self {
        let keep: Vec<String> = self.entries().into_iter().filter(|e| !paths::same_path(e, path)).collect();
        let mut doc = self.doc.clone();
        Self::write_entries(&mut doc, &keep);
        Self { doc }
    }

    #[must_use]
    pub fn with_sort(&self, sort: Sort) -> Self {
        let mut doc = self.doc.clone();
        ini::set(&mut doc, "browser", "sort", sort.key());
        Self { doc }
    }
}

/// The name a listed folder goes by: its last segment.
#[must_use]
pub fn entry_name(path: &str) -> &str {
    paths::leaf_of(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[recent]\r\np0=D:\\Mods\\Alpha\r\np1=D:\\Mods\\Beta\r\np2=\r\np3=\r\np4=\r\np5=\r\np6=\r\np7=\r\np8=\r\np9=\r\np10=\r\np11=\r\ncreate_parent=D:\\Mods\r\n[browser]\r\nsort=name\r\n";

    #[test]
    fn untouched_is_identical() {
        assert_eq!(ProjectList::parse(SAMPLE).to_text(), SAMPLE);
    }

    #[test]
    fn reads_the_list() {
        let list = ProjectList::parse(SAMPLE);
        assert_eq!(list.entries(), ["D:\\Mods\\Alpha", "D:\\Mods\\Beta"]);
        assert_eq!(list.create_parent(), Some("D:\\Mods"));
        assert_eq!(list.sort(), Sort::Name);
        assert_eq!(list.find_listed("d:/mods/beta/").as_deref(), Some("D:\\Mods\\Beta"));
        assert_eq!(list.find_listed("Beta"), None);
        assert_eq!(entry_name("D:\\Mods\\Beta"), "Beta");
    }

    #[test]
    fn opening_moves_to_the_front() {
        let list = ProjectList::parse(SAMPLE).opened("D:\\Mods\\Beta", "D:\\Mods");
        assert_eq!(list.entries(), ["D:\\Mods\\Beta", "D:\\Mods\\Alpha"]);
        assert_eq!(list.to_text(), SAMPLE.replace("p0=D:\\Mods\\Alpha\r\np1=D:\\Mods\\Beta", "p0=D:\\Mods\\Beta\r\np1=D:\\Mods\\Alpha"));
        // the same move again changes nothing
        assert_eq!(list.opened("D:\\Mods\\Beta", "D:\\Mods").to_text(), list.to_text());
    }

    #[test]
    fn twelve_at_most() {
        let mut list = ProjectList::default();
        for i in 0..14 {
            list = list.opened(&format!("D:\\P{i}"), "D:\\");
        }
        let entries = list.entries();
        assert_eq!(entries.len(), MAX_RECENT);
        assert_eq!(entries.first().map(String::as_str), Some("D:\\P13"));
        assert!(list.to_text().starts_with("[recent]\r\np0=D:\\P13\r\n"));
    }

    #[test]
    fn forgetting_and_sorting() {
        let list = ProjectList::parse(SAMPLE).without("D:/Mods/Alpha");
        assert_eq!(list.entries(), ["D:\\Mods\\Beta"]);
        assert!(list.to_text().contains("p0=D:\\Mods\\Beta\r\np1=\r\n"));
        assert_eq!(list.with_sort(Sort::Recent).sort(), Sort::Recent);
        assert!(list.with_sort(Sort::Recent).to_text().ends_with("sort=recent\r\n"));
    }
}
