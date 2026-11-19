//! What the quest editor reads of the open scene (its restrictors, its way paths and its
//! story ids): `gz-nq`'s
//! `SceneLookup` slot, filled when a project opens.

use std::sync::{Arc, Weak};

use gz_base::Rev;
use gz_editor::Editor;
use gz_nq::SceneLookup;
use gz_nq::pickers::PickerEntry;
use gz_scene::objects::SpawnEntity;
use gz_scene::{ObjClass, ObjData, SceneService, SceneState};

use crate::names::resolve_level;

/// The open scene, read live from its store.
pub struct LiveScene(pub Weak<Editor>);

impl LiveScene {
    fn scene(&self) -> Option<(Arc<Editor>, Arc<SceneService>)> {
        let ed = self.0.upgrade()?;
        let svc = ed.project_opt()?.services.get::<SceneService>()?;
        Some((ed, svc))
    }

    fn with<T>(&self, f: impl FnOnce(&SceneState) -> Vec<T>) -> Vec<T> {
        match self.scene() {
            Some((_, svc)) => {
                let snap = svc.store.snapshot();
                if snap.valid { f(&snap) } else { Vec::new() }
            }
            None => Vec::new(),
        }
    }
}

fn entry(id: &str, extra: String) -> PickerEntry {
    PickerEntry { id: id.to_owned(), name: String::new(), extra }
}

/// The story id of a spawn's custom data: `story_id = X` under `[story_object]`, or the bare
/// line right under the header.
#[must_use]
pub fn story_id(ini: &str) -> Option<String> {
    let at = ini.find("[story_object]")? + "[story_object]".len();
    for raw in ini[at..].split('\n') {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            return None;
        }
        let sid = match line.split_once('=') {
            Some((k, v)) if k.trim() == "story_id" => v.trim().to_owned(),
            Some(_) => return None,
            None => line.to_owned(),
        };
        return (!sid.is_empty()).then_some(sid);
    }
    None
}

impl SceneLookup for LiveScene {
    fn rev(&self) -> Rev {
        self.scene().map_or(Rev::ZERO, |(_, svc)| svc.store.rev())
    }

    fn restrictors(&self) -> Vec<PickerEntry> {
        self.with(|s| {
            s.of_class(ObjClass::SpawnPoint)
                .filter(|o| !o.name.is_empty() && o.ref_name() == Some("space_restrictor"))
                .map(|o| entry(&o.name, "scene".to_owned()))
                .collect()
        })
    }

    fn stories(&self) -> Vec<PickerEntry> {
        self.with(|s| {
            s.of_class(ObjClass::SpawnPoint)
                .filter_map(|o| {
                    let ObjData::Spawn(pt) = &o.data else { return None };
                    let ini = match pt.entity.as_ref()? {
                        SpawnEntity::Decoded(e) => e.custom_data()?.to_owned(),
                        _ => return None,
                    };
                    story_id(&ini).map(|sid| entry(&sid, format!("scene: {}", o.name)))
                })
                .collect()
        })
    }

    fn way_paths(&self) -> Vec<PickerEntry> {
        self.with(|s| {
            s.of_class(ObjClass::Way)
                .filter(|o| !o.name.is_empty())
                .map(|o| entry(&o.name, "scene".to_owned()))
                .collect()
        })
    }

    fn level(&self) -> Option<String> {
        let (ed, svc) = self.scene()?;
        let snap = svc.store.snapshot();
        let level = resolve_level(None, &snap.meta.level_name())?;
        let world = ed.project_opt()?.services.get::<gz_world::WorldService>()?.latest()?;
        world.levels.iter().any(|l| l.eq_ignore_ascii_case(&level)).then_some(level)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn story_ids_are_read_from_custom_data() {
        assert_eq!(
            story_id("[logic]\nactive = x\n[story_object]\nstory_id = esc_trader\n").as_deref(),
            Some("esc_trader")
        );
        assert_eq!(story_id("[story_object]\r\nesc_wolf\r\n").as_deref(), Some("esc_wolf"));
        assert_eq!(story_id("[story_object]\n[logic]\n"), None);
        assert_eq!(story_id("[logic]\n"), None);
    }
}
