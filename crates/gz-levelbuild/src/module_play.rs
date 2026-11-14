//! Play on a level the linked game has: the module, not a compiled level.
//!
//! The game composes a module's level layers over its own level on a new game (the game's
//! `MODDING.md` "World rules", `XMS_ARCHITECTURE.md` 7.2-7.4), so Play builds the module the way a
//! module build does into its root's `modules\<id>` (the game mounts `$fs_root$\modules`, and the
//! root is `$fs_root$` under `-fsltx`), and starts a new game ([`PARAMS`]). The actor starts at the
//! module's Player Start; a scene without one gets Play's own where the camera looks
//! ([`camera_start`]), in Play's copy of the module only. The new game's mode is the module's
//! ([`new_game_mode`]), written the way the new-game screen writes it (`play::write_game_options`).

use glam::Vec3;
use gz_editor::CameraPose;
use gz_scene::SceneState;
use gz_scene::library::Library;

use crate::play::PlayStart;

/// The game's command line for a new game on the modules of the root, after the root's own
/// `-fsltx`.
pub const PARAMS: &str = "-start server(all/single/alife/new) client(localhost) -nointro -noprefetch";

/// The mode a new game of the module is played in: none for one that targets the plain game
/// (`default`) or every mode (`any`, `mode = *`), the module's mode for `existing` and `new`; the
/// sentence that says what is missing when such a module names none yet.
pub fn new_game_mode(target: &str, mode: &str) -> Result<Option<String>, String> {
    let (target, mode) = (target.trim().to_ascii_lowercase(), mode.trim());
    match target.as_str() {
        "existing" | "new" if mode.is_empty() || mode == "*" => Err(format!(
            "The module targets {} mode and names none yet: pick it in Mod Settings, and Play plays the module in it.",
            if target == "new" { "a new" } else { "an existing" }
        )),
        "existing" | "new" => Ok(Some(mode.to_ascii_lowercase())),
        _ => Ok(None),
    }
}

/// Where a camera looks from its heading and pitch (the scene file's camera).
fn direction_of(hpb: Vec3) -> Vec3 {
    let (sh, ch) = hpb.x.sin_cos();
    let (sp, cp) = hpb.y.sin_cos();
    Vec3::new(-cp * sh, sp, cp * ch)
}

/// How far a camera's look finds the ground.
const REACH: f32 = 2000.0;

/// Where Play's Player Start stands: the ground under the centre of the view (`camera`, else the
/// camera the scene was saved with), else the ground below the camera, else the camera's point;
/// facing where the camera looks. Answers the start and which of the three it is.
#[must_use]
pub fn camera_start(
    scene: &SceneState,
    lib: &Library,
    camera: Option<&CameraPose>,
    level: &str,
) -> (PlayStart, &'static str) {
    let (eye, look) = match camera {
        Some(c) => (Vec3::from_array(c.position), c.direction.map(Vec3::from_array)),
        None => (scene.meta.camera_pos, Some(direction_of(scene.meta.camera_hpb))),
    };
    let look = look.filter(|d| d.length_squared() > 1e-6).map_or(Vec3::Z, Vec3::normalize);
    let pick = gz_scene::pick::ScenePick::build(scene, lib);
    let hit = |dir: Vec3| gz_scene::pick::ray_scene_objects(&pick, scene, lib, eye, dir, REACH, None);
    let (at, how) = match hit(look) {
        Some(h) => (h.point, "ground_in_view"),
        None => match hit(Vec3::NEG_Y) {
            Some(h) => (h.point, "ground_below"),
            None => (eye, "camera"),
        },
    };
    let heading = (-look.x).atan2(look.z);
    (PlayStart { level: level.to_owned(), position: at.to_array(), direction: [0.0, heading, 0.0] }, how)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_module_plays_in_its_mode_or_in_the_plain_game() {
        assert_eq!(new_game_mode("default", ""), Ok(None));
        assert_eq!(new_game_mode("any", "*"), Ok(None));
        assert_eq!(new_game_mode("existing", "Metro"), Ok(Some("metro".to_owned())));
        assert_eq!(new_game_mode("new", "my_story"), Ok(Some("my_story".to_owned())));
        let why = new_game_mode("existing", "").unwrap_err();
        assert!(why.contains("an existing mode and names none yet"), "{why}");
        assert!(new_game_mode("new", "*").unwrap_err().contains("a new mode"));
    }

    #[test]
    fn without_ground_the_start_is_the_camera_facing_where_it_looks() {
        let mut scene = SceneState::default();
        scene.meta.camera_pos = Vec3::new(10.0, 5.0, -3.0);
        let lib = Library::new(Vec::new());
        let cam = CameraPose { position: [1.0, 2.0, 3.0], direction: Some([1.0, 0.0, 0.0]), level: None };
        let (s, how) = camera_start(&scene, &lib, Some(&cam), "l01_escape");
        assert_eq!((s.level.as_str(), s.position, how), ("l01_escape", [1.0, 2.0, 3.0], "camera"));
        assert!((s.direction[1] + std::f32::consts::FRAC_PI_2).abs() < 1e-5, "{:?}", s.direction);
        // no viewport: the scene's own camera
        let (s, _) = camera_start(&scene, &lib, None, "l01_escape");
        assert_eq!(s.position, [10.0, 5.0, -3.0]);
    }
}
