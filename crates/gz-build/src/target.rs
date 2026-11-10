//! Where a build goes, and whether it may.
//!
//! A build replaces what is there, so a target that is a link - one made so the game runs the
//! working copy directly - or that is, holds or sits in the project would take the author's
//! sources with it. Judged on where the target would really be, before a single folder of it
//! is created.

use std::path::Path;

use gz_project::{is_editor_only_entry, paths};

/// The longest path the C++ editor built (`MAX_PATH`).
const MAX_PATH: usize = 260;

/// Forward slashes in, JSON-escaped doubled backslashes in - one clean path out: separators
/// made `\`, doubled ones collapsed (a UNC `\\server` prefix kept), no trailing one.
#[must_use]
pub fn normalize_path(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    for c in src.chars() {
        let c = if c == '/' { '\\' } else { c };
        if c == '\\' && out.len() > 1 && out.ends_with('\\') {
            continue;
        }
        out.push(c);
    }
    while out.ends_with('\\') {
        out.pop();
    }
    out
}

/// The one place a mod must NEVER land: inside the game's own gamedata. The whole point of
/// the module system is that a mod lives in its own folder and nothing ever merges into the
/// game tree - so a target that would put files there is refused, not warned about.
#[must_use]
pub fn inside_game_gamedata(path: &str, game_root: &str) -> bool {
    if game_root.is_empty() {
        return false;
    }
    let guard = format!("{game_root}\\gamedata");
    let n = guard.len();
    path.get(..n).is_some_and(|h| h.eq_ignore_ascii_case(&guard)) && matches!(path.as_bytes().get(n), None | Some(b'\\'))
}

/// Where `path` would really be once created: the deepest part of it that exists, resolved,
/// with the rest appended as written - so a refused build creates nothing.
#[must_use]
pub fn projected_real_path(path: &str) -> Option<String> {
    let mut head = path.to_owned();
    let mut tail = String::new();
    while !head.is_empty() {
        if Path::new(&head).is_dir()
            && let Some(mut real) = paths::real_path(&head)
        {
            if !tail.is_empty() {
                if !real.ends_with('\\') {
                    real.push('\\');
                }
                real.push_str(&tail);
            }
            return Some(real);
        }
        let cut = head.rfind('\\')?;
        let leaf = head.get(cut + 1..).unwrap_or("").to_owned();
        tail = if tail.is_empty() { leaf } else { format!("{leaf}\\{tail}") };
        head.truncate(cut);
    }
    None
}

/// The folder a build writes: `<target>\modules\<id>`, or `<target>\gamedata_<id>` for the flat
/// overlay - a module goes to `modules\`, NOT to `mods\`: that one is the folder JSGME manages,
/// and a module sitting there is a module JSGME offers to copy over the game. Blocking (it
/// resolves real paths): call it on the IO pool.
pub fn plan_target(target: &str, id: &str, flat: bool, project_root: &Path, game_root: &str) -> Result<String, String> {
    let target = normalize_path(target);
    if target.is_empty() {
        return Err("target folder not set".to_owned());
    }
    if target.len() + id.len() + 32 >= MAX_PATH {
        return Err("target path too long".to_owned());
    }
    let dst = if flat { format!("{target}\\gamedata_{id}") } else { format!("{target}\\modules\\{id}") };
    let game_root = paths::trim_trailing(paths::to_backslash(game_root));
    if inside_game_gamedata(&dst, &game_root) || inside_game_gamedata(&target, &game_root) {
        return Err("target is inside the game's gamedata - a mod must stay in its own folder (export to the game ROOT: the module goes to <game>\\modules\\<id>)".to_owned());
    }

    let real_dst = projected_real_path(&dst);
    let real_src = project_root.to_str().and_then(paths::real_path);
    let known = real_dst.as_ref().zip(real_src.as_ref());
    if std::fs::symlink_metadata(&dst).is_ok_and(|m| m.file_type().is_symlink()) {
        let to = known.map(|(d, _)| format!(" to {d}")).unwrap_or_default();
        return Err(format!("the target folder is a link{to} - a build replaces it, and would replace what it points at. Remove the link, or build to another folder"));
    }
    if let Some((real_dst, real_src)) = known {
        // inside the editor's own scratch is fine - the release stage lives in temp\, and no
        // build walks it
        let scratch = real_dst.len() > real_src.len()
            && paths::same_or_below(real_dst, real_src)
            && real_dst.get(real_src.len()..).map(|r| r.trim_start_matches('\\')).and_then(|r| r.split('\\').next()).is_some_and(is_editor_only_entry);
        if (paths::same_or_below(real_dst, real_src) && !scratch) || paths::same_or_below(real_src, real_dst) {
            return Err(format!("the target folder and the project overlap ({real_dst}) - the build would write into its own sources, or wipe them"));
        }
    }
    Ok(dst)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn paths_and_gamedata() {
        assert_eq!(normalize_path("D:/Games//DA/"), "D:\\Games\\DA");
        assert_eq!(normalize_path("\\\\server\\share\\\\x"), "\\\\server\\share\\x");
        assert!(inside_game_gamedata("D:\\DA\\GameData\\x", "D:\\DA"));
        assert!(inside_game_gamedata("D:\\DA\\gamedata", "D:\\DA"));
        assert!(!inside_game_gamedata("D:\\DA\\gamedata_od", "D:\\DA"));
        assert!(!inside_game_gamedata("D:\\DA\\gamedata", ""));
    }

    #[test]
    fn refusals_before_anything_is_made() {
        let base = std::env::temp_dir().join(format!("gz-build-target-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let project = base.join("project");
        let game = base.join("game");
        fs::create_dir_all(project.join("temp")).unwrap();
        fs::create_dir_all(&game).unwrap();
        let (p, g) = (project.to_str().unwrap().to_owned(), game.to_str().unwrap().to_owned());

        let dst = plan_target(&format!("{g}/"), "od", false, &project, &g).unwrap();
        assert_eq!(dst, format!("{g}\\modules\\od"));
        assert_eq!(plan_target(&g, "od", true, &project, &g).unwrap(), format!("{g}\\gamedata_od"));
        assert!(plan_target(&format!("{g}\\gamedata"), "od", false, &project, &g).unwrap_err().contains("gamedata"));
        let inside = plan_target(&p, "od", false, &project, &g).unwrap_err();
        assert!(inside.contains("overlap"), "{inside}");
        assert!(!project.join("modules").exists());
        // the module folder IS the project: a clean build there would wipe it
        let nested = base.join("modules").join("od");
        fs::create_dir_all(nested.join("sub")).unwrap();
        assert!(plan_target(&base.to_string_lossy(), "od", false, &nested, "").is_err());
        assert!(plan_target(&base.to_string_lossy(), "od", false, &nested.join("sub"), "").is_err());
        // the editor's scratch is fine: the release stage lives there
        assert!(plan_target(&format!("{p}\\temp\\release_stage_1"), "od", false, &project, &g).is_ok());
        assert!(plan_target("", "od", false, &project, &g).unwrap_err().contains("not set"));
        assert_eq!(plan_target(&"x".repeat(250), "od", false, &project, &g).unwrap_err(), "target path too long");
        fs::remove_dir_all(&base).unwrap();
    }
}
