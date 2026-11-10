//! The open scene's Player Starts (`CollectSceneStarts`): what `gz-world`'s `E201` counts for
//! the scene's level instead of that level's baked file - the build bakes the scene first, and
//! between two bakes the scene is what the author sees.

use gz_scene::math::get_xyz;
use gz_scene::objects::PointKind;
use gz_scene::{ObjClass, ObjData, SceneState};
use gz_world::Start;

/// The scene's Player Starts by their object names, on `level`.
#[must_use]
pub fn scene_starts(scene: &SceneState, level: &str) -> Vec<Start> {
    scene
        .of_class(ObjClass::SpawnPoint)
        .filter(|o| matches!(&o.data, ObjData::Spawn(s) if s.kind == PointKind::PlayerStart))
        .map(|o| {
            let p = o.position();
            let d = get_xyz(&o.world());
            Start { level: level.to_owned(), name: o.name.to_string(), source: "scene".to_owned(), position: p.to_array(), direction: d.to_array() }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_scene::Object;
    use gz_scene::objects::SpawnData;

    #[test]
    fn the_scene_names_its_starts() {
        let mut s = SceneState::default();
        let mut o = Object::new(ObjClass::SpawnPoint, "start", ObjData::Spawn(Box::new(SpawnData::of_kind(PointKind::PlayerStart))));
        o.xform.position = glam::Vec3::new(1.0, 2.0, 3.0);
        s.insert(o);
        s.insert(Object::new(ObjClass::SpawnPoint, "env", ObjData::Spawn(Box::new(SpawnData::of_kind(PointKind::EnvMod)))));
        let starts = scene_starts(&s, "l01_escape");
        assert_eq!(starts.len(), 1);
        assert_eq!((starts[0].name.as_str(), starts[0].level.as_str(), starts[0].source.as_str()), ("start", "l01_escape", "scene"));
        assert_eq!(starts[0].position, [1.0, 2.0, 3.0]);
    }
}
