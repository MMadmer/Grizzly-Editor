//! Where a level build writes, and whether it may empty that folder.
//!
//! The level path is free text of Level Settings; joined onto the levels folder as it is, an
//! absolute value would replace the base and `..` would climb out of it - and the build empties
//! the folder it lands on. So the name is ONE file-name component, and before anything is
//! wiped the folder must really (links resolved) lie below the levels folder and be no link.

use std::path::{Path, PathBuf};

use gz_project::paths;

/// `<levels>\<level>`, or the sentence that refuses the level path.
pub fn level_folder(levels: &Path, level: &str) -> Result<PathBuf, String> {
    match gz_base::file_name_problem(level) {
        Some(why) => Err(format!(
            "The level path '{level}' {why}. Level Settings > Build options > Level path names one folder under {}.",
            levels.display()
        )),
        None => Ok(levels.join(level)),
    }
}

/// Blocking (it resolves real paths): whether `dir` may be emptied. A folder that is not there
/// is fine - there is nothing to wipe.
pub fn check_wipe(levels: &Path, dir: &Path) -> Result<(), String> {
    match std::fs::symlink_metadata(dir) {
        Err(_) => return Ok(()),
        Ok(m) if m.file_type().is_symlink() => {
            return Err(format!(
                "The level folder {} is a link - a build empties it, and would empty what it points at. Remove the link.",
                dir.display()
            ));
        }
        Ok(_) => {}
    }
    let real = |p: &Path| p.to_str().and_then(paths::real_path);
    match (real(dir), real(levels)) {
        (Some(d), Some(base)) if d.len() > base.len() && paths::same_or_below(&d, &base) => Ok(()),
        (Some(d), Some(base)) => Err(format!(
            "The level folder is really {d}, which is not inside the levels folder {base}: it is not emptied."
        )),
        _ => Err(format!(
            "Where the level folder {} really is cannot be told: it is not emptied.",
            dir.display()
        )),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn a_level_path_is_one_folder_name() {
        let levels = Path::new("D:\\Editor\\gamedata\\levels");
        assert_eq!(level_folder(levels, "l01_escape").unwrap(), levels.join("l01_escape"));
        for bad in ["C:\\Windows", "..\\..\\x", "..", "a\\b", "a/b", "", "con", "x:y"] {
            let err = level_folder(levels, bad).unwrap_err();
            assert!(err.contains("one folder"), "{bad:?}: {err}");
        }
    }

    #[test]
    fn nothing_outside_the_levels_folder_or_behind_a_link_is_wiped() {
        let base = std::env::temp_dir().join(format!("gz-levelbuild-folder-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let levels = base.join("gamedata").join("levels");
        let outside = base.join("precious");
        fs::create_dir_all(levels.join("l01")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"x").unwrap();

        assert!(check_wipe(&levels, &levels.join("l01")).is_ok());
        assert!(check_wipe(&levels, &levels.join("not_built_yet")).is_ok());
        // the levels folder itself, or a folder beside it, is never emptied
        assert!(check_wipe(&levels, &levels).is_err());
        assert!(check_wipe(&levels, &outside).is_err());
        assert!(check_wipe(&levels, &levels.join("..").join("..").join("precious")).is_err());
        // a junction inside the levels folder that points out of it
        let link = levels.join("linked");
        let made = crate::process::console("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .output()
            .is_ok_and(|o| o.status.success());
        assert!(made, "a junction needs no privilege: mklink /J must work");
        let err = check_wipe(&levels, &link).unwrap_err();
        assert!(err.contains("is a link"), "{err}");
        fs::remove_dir(&link).unwrap();
        assert!(outside.join("keep.txt").is_file());
        fs::remove_dir_all(&base).unwrap();
    }
}
