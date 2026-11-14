//! What a path IS: the author's module, the editor's own scratch, sources the game never reads,
//! or nobody's content at all.

use std::io::{self, Write};
use std::path::Path;

/// The editor's scratch and captured preview, by their name in the project root: never shown as
/// content, never exported. The project file (`*.grizzly`) is the project itself and goes with
/// them. `project.ltx` is what the project folder kept there - this machine's paths: one that a
/// project still holds (it is converted and removed when the project is opened) never ships.
const EDITOR_ONLY: [&str; 9] = [
    "import",
    "temp",
    "logs",
    "savedgames",
    "screenshots",
    "backup",
    "releases",
    "preview.png",
    "project.ltx",
];

/// The guides at the root for those who work on the files without the editor (`AGENTS.md`, the
/// editor's; `CLAUDE.md`, an agent's own): the author's, never the module's.
const GUIDES: [&str; 2] = ["AGENTS.md", "CLAUDE.md"];

/// What the Windows shell leaves in folders it has shown.
const SHELL_LITTER: [&str; 4] = ["Thumbs.db", "ehthumbs.db", "ehthumbs_vista.db", "desktop.ini"];

/// Editor-only: an entry of the project root that is the editor's, not the module's.
#[must_use]
pub fn is_editor_only_entry(name_in_root: &str) -> bool {
    EDITOR_ONLY.iter().any(|n| n.eq_ignore_ascii_case(name_in_root))
        || crate::project::is_project_file_name(name_in_root)
}

/// Sources: shown and editable, but the game has no use for them and the export leaves them out.
#[must_use]
pub fn is_source_only_entry(name_in_root: &str) -> bool {
    name_in_root.eq_ignore_ascii_case("rawdata")
        || name_in_root.eq_ignore_ascii_case("Content")
        || GUIDES.iter().any(|n| n.eq_ignore_ascii_case(name_in_root))
}

/// The level build's own files in a level folder: what the compiler reads and the build keeps for
/// itself, never what the game does.
const LEVEL_BUILD_FILES: [&str; 9] = [
    "build.prj",
    "build.state",
    "build.report",
    "build.lights",
    "build.cform",
    "build.details",
    "build.err",
    "level_stat.tga",
    "level_stat.txt",
];

/// A file of a level folder the level build keeps for itself, by its name there.
#[must_use]
pub fn is_level_build_name(name: &str) -> bool {
    LEVEL_BUILD_FILES.iter().any(|n| n.eq_ignore_ascii_case(name))
}

/// A project-relative path (either separator) that is a level build's own file:
/// `gamedata\levels\<level>\build.prj` and the like. Never shipped.
#[must_use]
pub fn is_level_build_file(rel: &str) -> bool {
    let parts: Vec<&str> = rel.split(['\\', '/']).filter(|p| !p.is_empty()).collect();
    matches!(parts.as_slice(), [g, l, _, name]
        if g.eq_ignore_ascii_case("gamedata") && l.eq_ignore_ascii_case("levels") && is_level_build_name(name))
}

/// Tooling: version control, IDE and OS litter. Never shown, never shipped and never walked,
/// at ANY depth.
#[must_use]
pub fn is_tooling_entry(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    // every tool marks its own with a leading dot, and nothing the game reads is named that way
    if name.starts_with('.') {
        return true;
    }
    if SHELL_LITTER.iter().any(|n| n.eq_ignore_ascii_case(name)) {
        return true;
    }
    // an Office document open elsewhere keeps a lock file beside it
    name.starts_with("~$")
}

/// The `.gitignore` a git working copy gets once, for the editor's own files.
pub const GITIGNORE_TEXT: &str = "# Grizzly Editor: its own files, never part of the mod. Written once because\n\
# this project is a git repository - edit it freely, the editor will not again.\n\
\n\
# the Start page's picture of the project\n\
/preview.png\n\
# packaged releases - rebuilt from the sources, published on GitHub\n\
/releases/\n\
# scratch\n\
/backup/\n\
/temp/\n\
/logs/\n\
/import/\n\
/savedgames/\n\
/screenshots/\n\
\n\
# author key files: never published, never shipped (Mod > Submit to Mod Browser)\n\
*.xkey\n\
\n\
# Windows\n\
Thumbs.db\n\
desktop.ini\n";

/// A project that is a git working copy (`.git` a folder in a clone, a file in a worktree or a
/// submodule) and has no `.gitignore` gets [`GITIGNORE_TEXT`]. An author's own file is never
/// touched, one that appears meanwhile included. `Ok(true)` when it was written.
pub fn ensure_gitignore(root: &Path) -> io::Result<bool> {
    if std::fs::symlink_metadata(root.join(".git")).is_err() {
        return Ok(false);
    }
    let ignore = root.join(".gitignore");
    if std::fs::symlink_metadata(&ignore).is_ok() {
        return Ok(false);
    }
    let mut file = match std::fs::OpenOptions::new().write(true).create_new(true).open(&ignore) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return Ok(false),
        Err(e) => return Err(e),
    };
    file.write_all(GITIGNORE_TEXT.as_bytes())?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_folders_build_files_are_the_builds_own() {
        for p in [
            "gamedata\\levels\\l01_escape\\build.prj",
            "gamedata/levels/l01_escape/build.state",
            "GameData\\Levels\\x\\BUILD.LIGHTS",
            "gamedata\\levels\\x\\level_stat.tga",
        ] {
            assert!(is_level_build_file(p), "{p}");
        }
        for p in [
            "gamedata\\levels\\l01_escape\\level",
            "gamedata\\levels\\l01_escape\\build_details.dds",
            "gamedata\\levels\\l01_escape\\level.ai",
            "gamedata\\build.prj",
            "levels\\x\\build.prj",
            "gamedata\\levels\\x\\meshes\\build.prj",
        ] {
            assert!(!is_level_build_file(p), "{p}");
        }
    }

    #[test]
    fn tooling_at_any_depth() {
        for n in [
            ".git",
            ".gitignore",
            ".vscode",
            ".DS_Store",
            "Thumbs.db",
            "THUMBS.DB",
            "ehthumbs_vista.db",
            "Desktop.ini",
            "~$report.docx",
        ] {
            assert!(is_tooling_entry(n), "{n}");
        }
        for n in ["", "gamedata", "thumbs.dbx", "~report", "mod.ltx"] {
            assert!(!is_tooling_entry(n), "{n}");
        }
    }

    #[test]
    fn editor_and_source_entries() {
        assert!(is_editor_only_entry("Releases"));
        assert!(is_editor_only_entry("Delivery.grizzly"));
        assert!(!is_editor_only_entry(".grizzly"));
        assert!(!is_editor_only_entry("mod.ltx"));
        assert!(is_source_only_entry("RAWDATA"));
        assert!(is_source_only_entry("agents.md"));
        assert!(is_source_only_entry("CLAUDE.md"));
        assert!(is_source_only_entry("content"));
        assert!(!is_source_only_entry("gamedata"));
    }

    #[test]
    fn gitignore_written_once() {
        let dir = std::env::temp_dir().join(format!("gz-project-gitignore-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!ensure_gitignore(&dir).unwrap());
        std::fs::create_dir(dir.join(".git")).unwrap();
        assert!(ensure_gitignore(&dir).unwrap());
        assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), GITIGNORE_TEXT);
        // the project file is the author's and goes into the repository
        assert!(!GITIGNORE_TEXT.contains("grizzly") && !GITIGNORE_TEXT.contains("project.ltx"));
        std::fs::write(dir.join(".gitignore"), "mine\n").unwrap();
        assert!(!ensure_gitignore(&dir).unwrap());
        assert_eq!(std::fs::read_to_string(dir.join(".gitignore")).unwrap(), "mine\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
