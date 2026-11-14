//! The kinds of thing a level holds, as the Level tab shows them: one icon each, the name of
//! one (`ObjClass::desc`, "Spawn Point") and of several (`ObjClass::label_plural`, "Spawn
//! Points"), and the order the World Outliner lists them in - by that name, so an author finds
//! a kind where its name says, never by an internal number.

use gz_scene::ObjClass;
use gz_ui::Icon;

/// The icon of a kind.
#[must_use]
pub fn icon(class: ObjClass) -> Icon {
    match class {
        ObjClass::Group => Icon::Group,
        ObjClass::Glow => Icon::Glow,
        ObjClass::SceneObject => Icon::SceneObject,
        ObjClass::Light => Icon::Light,
        ObjClass::Shape => Icon::Shape,
        ObjClass::SoundSrc => Icon::SoundSource,
        ObjClass::SpawnPoint => Icon::Spawn,
        ObjClass::Way => Icon::Way,
        ObjClass::Sector => Icon::Sector,
        ObjClass::Portal => Icon::Portal,
        ObjClass::SoundEnv => Icon::SoundEnv,
        ObjClass::Particles => Icon::Particles,
        ObjClass::DetailObjects => Icon::DetailObjects,
        ObjClass::AiMap => Icon::AIMap,
        ObjClass::Wallmark => Icon::Wallmark,
        ObjClass::FogVolume => Icon::FogVolume,
    }
}

/// What one is called, Title Case: "Sound Source".
#[must_use]
pub fn title(class: ObjClass) -> &'static str {
    class.desc()
}

/// What several are called, Title Case: the Outliner's kind rows, "Spawn Points".
#[must_use]
pub fn plural(class: ObjClass) -> &'static str {
    class.label_plural()
}

/// The kinds of placed objects (the painted layers are not listed), ordered by their name.
#[must_use]
pub fn listed() -> Vec<ObjClass> {
    let mut out: Vec<ObjClass> = ObjClass::ALL.iter().copied().filter(|c| c.is_object_tool()).collect();
    out.sort_by_key(|c| plural(*c));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_are_named_and_ordered_by_their_names() {
        let l = listed();
        assert_eq!(l.first(), Some(&ObjClass::FogVolume));
        let names: Vec<&str> = l.iter().map(|c| plural(*c)).collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted);
        assert!(!l.contains(&ObjClass::AiMap));
        assert_eq!(title(ObjClass::SoundEnv), "Sound Environment");
        // every kind its own icon
        let mut icons: Vec<Icon> = ObjClass::ALL.iter().map(|c| icon(*c)).collect();
        icons.dedup();
        assert_eq!(icons.len(), ObjClass::ALL.len());
    }
}
