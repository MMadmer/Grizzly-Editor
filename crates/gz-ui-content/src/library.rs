//! The Library source: what can be brought into the project or placed, by kind
//! ([`crate::model::KINDS`]), each kind one catalog merged from where it lives - the linked game's
//! index and the editor's library (Preferences > Library). Built on the pools; a frame never
//! waits for one.
//!
//! - Objects, Groups, Base Scenes: the library's `rawdata` folders (the game holds none).
//! - Textures, Sounds: the game's `textures` / `sounds` first, then what only the library has -
//!   the order a scene resolves them in (`docs/ASSETS.md` section 9).
//! - Particles: the effects and groups of the library's `particles.xr` (`file.list`).
//! - Spawn Sections: the game's sections that declare `$spawn`, in the folders of their `$spawn`
//!   path as the Place panel groups them, an item by its in-game name (`game.lookup`).

use std::path::Path;
use std::sync::Arc;

use gz_base::{Origin as CallOrigin, SmolStr};
use gz_editor::{Editor, Value, json};
use gz_project::{FileTree, Project};
use rustc_hash::FxHashMap;

use crate::catalog::{Catalog, FileItem, Origin};
use crate::model::{self, Source};

/// What a library folder holds, walked; a missing folder is an empty catalog that says so.
fn from_library(root: &Path, category: usize) -> Catalog {
    let folder = model::kind(category).map_or("", |k| k.library_folder);
    let dir = root.join(folder);
    if dir.is_dir() {
        Catalog::from_library_dir(&FileTree::scan(&dir), category)
    } else {
        let mut c = Catalog::empty(Source::Library, category);
        c.disk_root = Some(dir.clone());
        c.note =
            Some(format!("The editor's library has no {} folder (Preferences > Library).", dir.display()));
        c
    }
}

/// The game's files of a kind beside the library's: the game's win a path both hold.
fn merge_game(mut lib: Catalog, files: Option<Arc<gz_vfs::GameFiles>>, category: usize) -> Catalog {
    let Some(k) = model::kind(category) else { return lib };
    let Some(files) = files else { return lib };
    let prefix = format!("{}/", k.game_folder);
    let mut games: Vec<FileItem> = Vec::new();
    for f in files.iter() {
        let Some(rest) = f.path.strip_prefix(&prefix) else { continue };
        if !rest.to_ascii_lowercase().ends_with(k.ext) {
            continue;
        }
        let rel = rest.replace('/', "\\");
        if lib.file(&rel).is_some() {
            continue;
        }
        games.push(FileItem {
            path: SmolStr::new(&rel),
            size: f.size,
            asset: None,
            origin: Origin::Game,
            label: None,
            note: None,
            companions: Vec::new(),
            published: None,
        });
    }
    // a path both hold is the game's: the library keeps what the game does not have
    let game_paths: rustc_hash::FxHashSet<String> = games.iter().map(|g| g.path.to_lowercase()).collect();
    let mut out = Catalog::empty(Source::Library, category);
    out.disk_root = lib.disk_root.take();
    out.note = lib.note.take();
    for f in lib.files().cloned().collect::<Vec<_>>() {
        if !game_paths.contains(&f.path.to_lowercase()) {
            out.add(f);
        }
    }
    for g in games {
        out.add(g);
    }
    out.game_prefix = k.game_folder.to_owned();
    out.game = Some(files);
    out.sort();
    out
}

async fn call(editor: &Arc<Editor>, name: &str, args: Value) -> Result<Value, String> {
    let Value::Object(map) = args else { return Err("bad arguments".to_owned()) };
    let reply = editor.call(name, map, CallOrigin::Ui).await;
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(reply)
    } else {
        Err(reply.get("error").and_then(Value::as_str).unwrap_or("it did not answer").to_owned())
    }
}

/// The entries of a lookup: `(id, name, extra)`.
fn entries(reply: &Value) -> Vec<(String, String, String)> {
    reply
        .get("entries")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|e| {
                    let s = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
                    (s("id"), s("name"), s("extra"))
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn spawn_sections(editor: &Arc<Editor>) -> Catalog {
    let mut c = Catalog::empty(Source::Library, model::SPAWNS);
    let roster =
        match call(editor, "game.lookup", json!({ "type": "spawn_section", "limit": 1_000_000 })).await {
            Ok(r) => entries(&r),
            Err(e) => {
                c.note = Some(format!("The spawn sections could not be read: {e}"));
                return c;
            }
        };
    // items by the name the player reads; a list that is not there leaves the section ids
    let names: FxHashMap<String, String> =
        match call(editor, "game.lookup", json!({ "type": "item_section", "limit": 1_000_000 })).await {
            Ok(r) => entries(&r)
                .into_iter()
                .filter(|(id, name, _)| !name.is_empty() && name != id)
                .map(|(id, name, _)| (id.to_lowercase(), name))
                .collect(),
            Err(_) => FxHashMap::default(),
        };
    let exec = editor.exec.clone();
    exec.cpu(move || {
        // a name several sections share (a weapon and its variants) says which one it is
        let mut uses: FxHashMap<&str, usize> = FxHashMap::default();
        for (section, _, _) in &roster {
            if let Some(n) = names.get(&section.to_lowercase()) {
                *uses.entry(n.as_str()).or_default() += 1;
            }
        }
        let label_of = |section: &str| {
            let n = names.get(&section.to_lowercase())?;
            Some(if uses.get(n.as_str()).copied().unwrap_or(0) > 1 {
                format!("{n} ({section})")
            } else {
                n.clone()
            })
        };
        for (section, group, class) in &roster {
            let path = if group.is_empty() { section.clone() } else { format!("{group}\\{section}") };
            let mut item = FileItem {
                path: SmolStr::new(&path),
                size: 0,
                asset: None,
                origin: Origin::None,
                label: label_of(section).map(SmolStr::new),
                note: Some(SmolStr::new(if class.is_empty() {
                    format!("Section {section}")
                } else {
                    format!("Section {section} \u{b7} class {class}")
                })),
                companions: Vec::new(),
                published: None,
            };
            if item.label.as_deref() == Some(section.as_str()) {
                item.label = None;
            }
            c.add(item);
        }
        c.sort();
        c
    })
    .await
}

async fn particles(editor: &Arc<Editor>) -> Catalog {
    let mut c = Catalog::empty(Source::Library, model::PARTICLES);
    match call(editor, "file.list", json!({ "source": "library", "kind": "particles", "limit": 1_000_000 }))
        .await
    {
        Ok(r) => {
            for f in r.get("files").and_then(Value::as_array).into_iter().flatten() {
                if let Some(p) = f.get("path").and_then(Value::as_str) {
                    c.add(FileItem {
                        path: SmolStr::new(p.replace('/', "\\")),
                        size: 0,
                        asset: None,
                        origin: Origin::None,
                        label: None,
                        note: None,
                        companions: Vec::new(),
                        published: None,
                    });
                }
            }
            if c.file_count() == 0 {
                c.note =
                    Some("The editor's library holds no particles.xr, or it lists no effect.".to_owned());
            }
        }
        Err(e) => c.note = Some(format!("The particles could not be read: {e}")),
    }
    c.sort();
    c
}

/// The game's revision a kind's catalog depends on (0 for the kinds that never read the game).
#[must_use]
pub(crate) fn reads_game(category: usize) -> bool {
    matches!(category, model::TEXTURES | model::SOUNDS | model::SPAWNS)
}

/// Builds the catalog of a Library kind. The game is read (and mounted, the first time) for the
/// kinds that hold its files.
pub(crate) async fn build(editor: &Arc<Editor>, category: usize) -> Catalog {
    let root = gz_project::library_root(editor);
    match category {
        model::PARTICLES => particles(editor).await,
        model::SPAWNS => spawn_sections(editor).await,
        model::TEXTURES | model::SOUNDS => {
            let game = match Project::of(editor) {
                Ok((_, p)) if !p.game_root().is_empty() => editor.exec.io(move || p.game_files().ok()).await,
                _ => None,
            };
            let lib = editor.exec.io(move || from_library(&root, category)).await;
            let unlinked = game.is_none();
            let mut cat = editor.exec.cpu(move || merge_game(lib, game, category)).await;
            if unlinked && cat.note.is_none() {
                cat.note = Some("No game is linked: only the editor's library is listed.".to_owned());
            }
            cat
        }
        _ => editor.exec.io(move || from_library(&root, category)).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_entries_read_their_lookup() {
        let reply = json!({ "ok": true, "entries": [{ "id": "wpn_pm", "name": "weapons\\pistols", "extra": "WP_PM" }] });
        assert_eq!(entries(&reply), [("wpn_pm".into(), "weapons\\pistols".into(), "WP_PM".into())]);
        assert!(reads_game(model::TEXTURES) && !reads_game(model::OBJECTS));
    }
}
