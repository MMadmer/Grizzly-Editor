//! The file work of the area. Blocking: call it on the IO pool.

use std::io;
use std::path::Path;

use gz_base::RelPath;
use gz_project::FileTree;

use crate::commands::scan_script_functions;
use crate::doc::ManifestDoc;
use crate::exclude::outside_build;
use crate::model::{FILE, Manifest, scaffold_text};

/// `<root>\mod.ltx` as text (UTF-8, a BOM skipped, anything that is not UTF-8 read as cp1251 -
/// the reading the game gives the file); `Ok(None)` when there is none.
pub fn read_manifest_file(root: &Path) -> io::Result<Option<String>> {
    Ok(gz_editor::fsx::read_opt(&root.join(FILE))?.map(|bytes| gz_base::text::decode_guess(&bytes).into_owned()))
}

/// The project scaffold: a project without `mod.ltx` gets [`scaffold_text`]. Never overwrites
/// a file, one that appears meanwhile included. `Ok(true)` when it was written.
pub fn scaffold(root: &Path, project_name: &str) -> io::Result<bool> {
    use std::io::Write;
    let path = root.join(FILE);
    if std::fs::symlink_metadata(&path).is_ok() {
        return Ok(false);
    }
    let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e),
    };
    file.write_all(scaffold_text(project_name).as_bytes())?;
    Ok(true)
}

/// What a console command may call: `function name(` and `name = function(` of the project's
/// `gamedata\scripts\*.script` as `<script>.<function>`, sorted. The game's script namespaces
/// are the files right in `scripts\`, never a folder below it; a script the build leaves out has
/// none. The folder is listed from `tree`; only the scripts themselves are read.
#[must_use]
pub fn script_functions(root: &Path, tree: &FileTree, exclude: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(dir_rel) = RelPath::new("gamedata/scripts") else { return out };
    let Some(dir) = tree.dir(&dir_rel) else { return out };
    for entry in dir.entries.iter().filter(|e| e.is_file()) {
        let name = entry.name.as_str();
        let Some(stem) = name.len().checked_sub(7).filter(|&n| n > 0).and_then(|n| name.get(..n)) else { continue };
        if !name.get(stem.len()..).is_some_and(|ext| ext.eq_ignore_ascii_case(".script")) {
            continue;
        }
        let rel = format!("gamedata\\scripts\\{name}");
        if outside_build(exclude, &rel) {
            continue;
        }
        if let Ok(text) = gz_editor::fsx::read_text(&dir_rel.to_fs(root).join(name)) {
            scan_script_functions(&text, stem, &mut out);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The `[commands]` the modules of the linked game declare, as `(command, module id)`: both
/// roots the game reads modules from, `modules\` and `mods\`; the module `self_id` left out.
#[must_use]
pub fn installed_commands(game_root: &Path, self_id: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for sub in ["modules", "mods"] {
        let Ok(listing) = std::fs::read_dir(game_root.join(sub)) else { continue };
        let mut folders: Vec<(String, std::path::PathBuf)> = listing
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok().map(|n| (n, e.path())))
            .filter(|(n, p)| !n.starts_with('.') && p.is_dir())
            .collect();
        folders.sort_by_key(|(n, _)| n.to_ascii_lowercase());
        for (folder, path) in folders {
            let Ok(Some(text)) = read_manifest_file(&path) else { continue };
            let other = Manifest::parse(&ManifestDoc::parse(&text));
            let id = if other.id.is_empty() { folder } else { other.id };
            if id.eq_ignore_ascii_case(self_id) {
                continue;
            }
            out.extend(other.commands.into_iter().filter(|c| c.declared).map(|c| (c.name, id.clone())));
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;

    fn temp(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gz-mod-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn scaffold_never_overwrites() {
        let dir = temp("scaffold");
        assert!(scaffold(&dir, "Night Run").unwrap());
        assert_eq!(read_manifest_file(&dir).unwrap().unwrap(), scaffold_text("Night Run"));
        fs::write(dir.join(FILE), "[module]\nid = mine\n").unwrap();
        assert!(!scaffold(&dir, "Night Run").unwrap());
        assert_eq!(read_manifest_file(&dir).unwrap().unwrap(), "[module]\nid = mine\n");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scripts_and_installed_commands() {
        let dir = temp("scripts");
        fs::create_dir_all(dir.join("gamedata\\scripts\\deep")).unwrap();
        fs::write(dir.join("gamedata\\scripts\\od.script"), "function status()\nfunction aaa()\n").unwrap();
        fs::write(dir.join("gamedata\\scripts\\wip.script"), "function hidden()\n").unwrap();
        fs::write(dir.join("gamedata\\scripts\\deep\\x.script"), "function no()\n").unwrap();
        let tree = FileTree::scan(&dir);
        assert_eq!(script_functions(&dir, &tree, &["gamedata/scripts/wip.script".to_owned()]), ["od.aaa", "od.status"]);

        let game = dir.join("game");
        fs::create_dir_all(game.join("modules\\other")).unwrap();
        fs::create_dir_all(game.join("modules\\self")).unwrap();
        fs::create_dir_all(game.join("mods\\noid")).unwrap();
        fs::write(game.join("modules\\other\\mod.ltx"), "[module]\nid = other\n[commands]\nnight = o.n\n").unwrap();
        fs::write(game.join("modules\\self\\mod.ltx"), "[module]\nid = od\n[commands]\nstatus = od.s\n").unwrap();
        fs::write(game.join("mods\\noid\\mod.ltx"), "[commands]\nfly = n.f\n").unwrap();
        assert_eq!(installed_commands(&game, "OD"), [("night".to_owned(), "other".to_owned()), ("fly".to_owned(), "noid".to_owned())]);
        fs::remove_dir_all(&dir).unwrap();
    }
}
