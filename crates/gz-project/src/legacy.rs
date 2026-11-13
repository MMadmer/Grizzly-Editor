//! `project.ltx`, the bookkeeping file projects carried before the project file: read once, when
//! the project is first opened by this editor, for its game link, its export target and its last
//! scene - then deleted. Nothing else reads or writes it.

use gz_ltx::LtxDoc;

use crate::ini;

pub const FILE: &str = "project.ltx";

/// What an old `project.ltx` held that is worth keeping.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Legacy {
    /// The linked game's folder, without trailing separators.
    pub game_root: String,
    /// Where Build Into Game wrote last.
    pub export_target: String,
    /// The scene to reopen, project-relative with `/` (an absolute one only when it lies inside
    /// the project - [`crate::project::scene_in_project`] decides at the open).
    pub last_scene: String,
}

impl Legacy {
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let doc = LtxDoc::parse(text);
        let get = |s: &str, k: &str| ini::get(&doc, s, k).unwrap_or_default().trim().to_owned();
        Self {
            game_root: get("project", "game_root").trim_end_matches(['\\', '/']).to_owned(),
            export_target: get("xms", "export_target"),
            last_scene: get("project", "last_scene"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_an_old_file_held() {
        let old = Legacy::parse(
            "[project]\r\nname=Delivery\r\nsdk_root=D:\\Editor\\\r\ngame_root=D:\\Games\\DA\\\r\nlast_scene=rawdata\\levels\\a.level\r\n\r\n[xms]\r\nexport_target=D:\\Out\r\n",
        );
        assert_eq!(
            old,
            Legacy {
                game_root: "D:\\Games\\DA".into(),
                export_target: "D:\\Out".into(),
                last_scene: "rawdata\\levels\\a.level".into(),
            }
        );
        assert_eq!(Legacy::parse(""), Legacy::default());
    }
}
