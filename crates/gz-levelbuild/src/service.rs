//! What the level build keeps while a project is open: the scene's derived data and what it
//! was built from, and the process that runs.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use gz_editor::{JobId, ProjectSession};
use gz_fmt_level::{CForm, LevelGraph};
use gz_scene::{ObjClass, SceneState};
use parking_lot::Mutex;
use serde::Serialize;

use crate::graph::LevelGameGraph;

/// What holds the `level_run` job: the level compiler in the editor's process, or the game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Running {
    Compile,
    Game,
}

impl Running {
    /// The name `level_build status` answers in `running`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Compile => "gz-lc",
            Self::Game => "Xr3DA.exe",
        }
    }
}

/// What of the scene's derived data is missing or older than the scene: the status bar's
/// Rebuild chip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Rebuild {
    pub cform: bool,
    pub ai_map: bool,
    pub game_graph: bool,
}

impl Rebuild {
    #[must_use]
    pub fn any(&self) -> bool {
        self.cform || self.ai_map || self.game_graph
    }

    /// The chip's text: what `NEED REBUILD` listed.
    #[must_use]
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        if self.cform {
            parts.push("Collision");
        }
        if self.ai_map {
            parts.push("AI Map");
        }
        if self.game_graph {
            parts.push("Game Graph");
        }
        parts.join(", ")
    }
}

/// A built piece and the fingerprint of the scene data it was built from.
#[derive(Clone)]
pub(crate) struct Built<T> {
    pub key: u64,
    pub value: Arc<T>,
}

#[derive(Default)]
pub(crate) struct State {
    pub cform: Option<Built<CForm>>,
    pub level_graph: Option<Built<LevelGraph>>,
    pub game_graph: Option<Built<LevelGameGraph>>,
}

/// The level build of the open project.
#[derive(Default)]
pub struct LevelBuild {
    pub(crate) state: Mutex<State>,
    /// The compile or the game that runs, and its job.
    pub(crate) running: Mutex<Option<(Running, JobId)>>,
}

fn fingerprint<'a>(items: impl Iterator<Item = &'a Arc<gz_scene::Object>>) -> u64 {
    let mut h = DefaultHasher::new();
    for o in items {
        o.id.hash(&mut h);
        (Arc::as_ptr(o) as usize).hash(&mut h);
    }
    h.finish()
}

/// What the collision form is made of: the scene objects.
pub(crate) fn cform_key(s: &SceneState) -> u64 {
    fingerprint(s.of_class(ObjClass::SceneObject))
}

/// What the level graph is made of: the AI Map tool's nodes.
pub(crate) fn ai_key(s: &SceneState) -> u64 {
    let mut h = DefaultHasher::new();
    s.bulk.ai_map.as_ref().map(|a| Arc::as_ptr(a) as usize).hash(&mut h);
    h.finish()
}

/// What the game graph is made of: the level graph and the spawn points (its graph points).
pub(crate) fn graph_key(s: &SceneState) -> u64 {
    let mut h = DefaultHasher::new();
    ai_key(s).hash(&mut h);
    fingerprint(s.of_class(ObjClass::SpawnPoint)).hash(&mut h);
    h.finish()
}

impl LevelBuild {
    #[must_use]
    pub fn of(session: &ProjectSession) -> Arc<Self> {
        session.services.get_or_init(Self::default)
    }

    /// What needs building again for the scene there is now.
    #[must_use]
    pub fn rebuild(&self, scene: &SceneState) -> Rebuild {
        let st = self.state.lock();
        let stale = |key: Option<u64>, now: u64| key != Some(now);
        Rebuild {
            cform: stale(st.cform.as_ref().map(|b| b.key), cform_key(scene)),
            ai_map: stale(st.level_graph.as_ref().map(|b| b.key), ai_key(scene)),
            game_graph: stale(st.game_graph.as_ref().map(|b| b.key), graph_key(scene)),
        }
    }

    /// The compile or the game that runs.
    #[must_use]
    pub fn running(&self) -> Option<Running> {
        self.running.lock().map(|(p, _)| p)
    }
}
