//! The project list, `_projects.ini` in the editor's home: the Start page's memory and nothing
//! more - `[recent] p0..p11` (the last opened first) with the project's id beside each
//! (`id0..id11`), `create_parent` (the New Project form's location) and `[browser] sort`. A
//! project that is not on it opens the same; one that moved shows as Missing until it is opened
//! from its new place, which takes the old entry's place by the id.
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
    /// The folder holds a project file or `mod.ltx`.
    Project,
    /// The folder does not exist.
    Missing,
    /// A folder with neither in it.
    NotAProject,
}

impl EntryState {
    /// As `project.list` names it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            EntryState::Project => "project",
            EntryState::Missing => "missing",
            EntryState::NotAProject => "not_a_project",
        }
    }
}

/// A project is a folder with a project file (`*.grizzly`) or with `mod.ltx` (a fresh clone of a
/// mod's repository; opening it writes the project file). A folder with an old `project.ltx`
/// that no editor has opened since is one too: the open converts it.
#[must_use]
pub fn has_manifest(folder: &Path) -> bool {
    std::fs::symlink_metadata(folder.join("mod.ltx")).is_ok()
        || std::fs::symlink_metadata(folder.join(crate::legacy::FILE)).is_ok()
        || !crate::project::project_files(folder).is_empty()
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
        self.entries_with_ids().into_iter().map(|(p, _)| p).collect()
    }

    /// The listed folders with the id of the project each held when it was opened (`""` when
    /// the list does not know it).
    #[must_use]
    pub fn entries_with_ids(&self) -> Vec<(String, String)> {
        (0..MAX_RECENT)
            .filter_map(|i| {
                let path = ini::get(&self.doc, "recent", &format!("p{i}")).filter(|v| !v.is_empty())?;
                let id = ini::get(&self.doc, "recent", &format!("id{i}")).unwrap_or_default();
                Some((path.to_owned(), id.to_owned()))
            })
            .collect()
    }

    /// The id the list knows for a listed folder.
    #[must_use]
    pub fn id_of(&self, path: &str) -> Option<String> {
        self.entries_with_ids().into_iter().find(|(p, _)| paths::same_path(p, path)).map(|(_, id)| id)
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

    fn write_entries(doc: &mut LtxDoc, entries: &[(String, String)]) {
        for i in 0..MAX_RECENT {
            let (path, id) = entries.get(i).map_or(("", ""), |(p, id)| (p.as_str(), id.as_str()));
            ini::set(doc, "recent", &format!("p{i}"), path);
            // an id is written beside its entry, and only where the list knows ids at all
            if !id.is_empty() || ini::get(doc, "recent", &format!("id{i}")).is_some() {
                ini::set(doc, "recent", &format!("id{i}"), id);
            }
        }
    }

    /// The list after the project `id` opened from `project`: it floats to the top, its
    /// duplicates collapse - another spelling of the folder, and the entry of the same project
    /// at a place it moved away from - the rest keep their order (twelve at most), and the
    /// form's location is written along.
    #[must_use]
    pub fn opened(&self, project: &str, id: &str, create_parent: &str) -> Self {
        let mut list = vec![(project.to_owned(), id.to_owned())];
        list.extend(self.entries_with_ids().into_iter().filter(|(p, other)| {
            !p.eq_ignore_ascii_case(project) && (id.is_empty() || !other.eq_ignore_ascii_case(id))
        }));
        list.truncate(MAX_RECENT);
        let mut doc = self.doc.clone();
        Self::write_entries(&mut doc, &list);
        ini::set(&mut doc, "recent", "create_parent", create_parent);
        Self { doc }
    }

    /// The list without `path` (every spelling of it).
    #[must_use]
    pub fn without(&self, path: &str) -> Self {
        let keep: Vec<(String, String)> =
            self.entries_with_ids().into_iter().filter(|(e, _)| !paths::same_path(e, path)).collect();
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
        let list = ProjectList::parse(SAMPLE).opened("D:\\Mods\\Beta", "", "D:\\Mods");
        assert_eq!(list.entries(), ["D:\\Mods\\Beta", "D:\\Mods\\Alpha"]);
        assert_eq!(
            list.to_text(),
            SAMPLE.replace(
                "p0=D:\\Mods\\Alpha\r\np1=D:\\Mods\\Beta",
                "p0=D:\\Mods\\Beta\r\np1=D:\\Mods\\Alpha"
            )
        );
        // the same move again changes nothing
        assert_eq!(list.opened("D:\\Mods\\Beta", "", "D:\\Mods").to_text(), list.to_text());
    }

    #[test]
    fn twelve_at_most() {
        let mut list = ProjectList::default();
        for i in 0..14 {
            list = list.opened(&format!("D:\\P{i}"), "", "D:\\");
        }
        let entries = list.entries();
        assert_eq!(entries.len(), MAX_RECENT);
        assert_eq!(entries.first().map(String::as_str), Some("D:\\P13"));
        assert!(list.to_text().starts_with("[recent]\r\np0=D:\\P13\r\n"));
    }

    #[test]
    fn a_project_that_moved_takes_its_old_entrys_place() {
        let list = ProjectList::default().opened("D:\\Mods\\Alpha", "aaaa", "D:\\Mods").opened(
            "D:\\Mods\\Beta",
            "bbbb",
            "D:\\Mods",
        );
        assert_eq!(list.id_of("d:/mods/alpha").as_deref(), Some("aaaa"));
        // Alpha moved to E:: one entry, at the new place
        let moved = list.opened("E:\\Alpha", "aaaa", "D:\\Mods");
        assert_eq!(
            moved.entries_with_ids(),
            [("E:\\Alpha".to_owned(), "aaaa".to_owned()), ("D:\\Mods\\Beta".to_owned(), "bbbb".to_owned())]
        );
        // forgetting keeps the ids with their entries
        let forgot = moved.without("E:\\Alpha");
        assert_eq!(forgot.entries_with_ids(), [("D:\\Mods\\Beta".to_owned(), "bbbb".to_owned())]);
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
