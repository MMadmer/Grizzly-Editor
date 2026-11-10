//! The tool classes as the Tools panel and the World Outliner name them (`kToolClasses` of
//! `UILeftBarForm.cpp`): one icon, a tile label and a title per class.

use gz_scene::ObjClass;
use gz_ui::Icon;

/// One tool class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToolClass {
    pub class: ObjClass,
    pub icon: Icon,
    /// What a tile shows: the title, or a short form where the title is wider than a tile of
    /// the 240 pt panel.
    pub label: &'static str,
    /// What the header ("<title> Tool"), the Outliner's class rows and the palette call it.
    pub title: &'static str,
}

const fn tc(class: ObjClass, icon: Icon, label: &'static str, title: &'static str) -> ToolClass {
    ToolClass { class, icon, label, title }
}

/// In the order of the Tools panel's tiles.
pub const TOOL_CLASSES: [ToolClass; 16] = [
    tc(ObjClass::SceneObject, Icon::SceneObject, "Object", "Object"),
    tc(ObjClass::Light, Icon::Light, "Light", "Light"),
    tc(ObjClass::SoundSrc, Icon::SoundSource, "Sound", "Sound Source"),
    tc(ObjClass::SoundEnv, Icon::SoundEnv, "Sound Env", "Sound Environment"),
    tc(ObjClass::Glow, Icon::Glow, "Glow", "Glow"),
    tc(ObjClass::Shape, Icon::Shape, "Shape", "Shape"),
    tc(ObjClass::SpawnPoint, Icon::Spawn, "Spawn", "Spawn"),
    tc(ObjClass::Way, Icon::Way, "Way", "Way"),
    tc(ObjClass::Sector, Icon::Sector, "Sector", "Sector"),
    tc(ObjClass::Portal, Icon::Portal, "Portal", "Portal"),
    tc(ObjClass::Group, Icon::Group, "Group", "Group"),
    tc(ObjClass::Particles, Icon::Particles, "Particles", "Particles"),
    tc(ObjClass::DetailObjects, Icon::DetailObjects, "Detail Obj.", "Detail Objects"),
    tc(ObjClass::AiMap, Icon::AIMap, "AI Map", "AI Map"),
    tc(ObjClass::Wallmark, Icon::Wallmark, "Wallmark", "Wallmark"),
    tc(ObjClass::FogVolume, Icon::FogVolume, "Fog", "Fog Volume"),
];

fn find(class: ObjClass) -> Option<&'static ToolClass> {
    TOOL_CLASSES.iter().find(|c| c.class == class)
}

/// `ToolClassIcon`.
#[must_use]
pub fn icon(class: ObjClass) -> Icon {
    find(class).map_or(Icon::SceneObject, |c| c.icon)
}

/// `ToolClassLabel`.
#[must_use]
pub fn label(class: ObjClass) -> &'static str {
    find(class).map_or("Object", |c| c.label)
}

/// `ToolClassTitle`.
#[must_use]
pub fn title(class: ObjClass) -> &'static str {
    find(class).map_or("Object", |c| c.title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_class_has_a_tile() {
        for c in ObjClass::ALL {
            assert!(find(c).is_some(), "{c:?}");
        }
        assert_eq!(title(ObjClass::SoundEnv), "Sound Environment");
        assert_eq!(label(ObjClass::DetailObjects), "Detail Obj.");
    }
}
