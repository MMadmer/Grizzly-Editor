//! The game's levels as what a module's scene lies over (`docs/GAME_LIBRARY.md`, 5).
//!
//! A scene that says `base = "<level>"` gets the level's own objects from here as it is read:
//! the same scene the library writes as text ([`crate::levels::scene_of`]), made from the
//! game's files as they are now, with the level's AI map (`level.ai`) as its painted layer and
//! every object marked the level's own. A level is made once and kept until one of its files
//! changes.

use std::sync::{Arc, Weak};

use gz_base::SmolStr;
use gz_editor::Editor;
use gz_entities::Settings;
use gz_fmt_level::{AiMapNode, AiParams, LevelGraph, NodeLayout, SceneAiMap, nodes_bbox, pv_decompress};
use gz_scene::base::{BaseLevels, BaseSource};
use gz_scene::model::SceneState;
use gz_scene::tools::AiMapPart;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::levels::{Spawn, level_names, level_stamp, scene_of};
use crate::service::GameLibrary;
use crate::state::Stamps;

/// The levels of the game there is now, each kept with what it was made from.
pub struct LibraryLevels {
    editor: Weak<Editor>,
    made: Mutex<FxHashMap<String, (String, Arc<SceneState>)>>,
    /// Per level, the turn at making it: scenes over one level checked together read the level
    /// once, the rest wait and take what the first made.
    making: Mutex<FxHashMap<String, Arc<Mutex<()>>>>,
}

/// A compiled AI map as the scene's painted layer: every node where the game has it, with its
/// plane and its links.
#[must_use]
pub fn ai_layer(graph: &LevelGraph) -> AiMapPart {
    let none = NodeLayout::of(graph.header.version).map(|l| l.invalid_link());
    let count = graph.nodes.len();
    let nodes: Vec<AiMapNode> = graph
        .nodes
        .iter()
        .map(|n| {
            let mut node = AiMapNode::new(graph.position(n), pv_decompress(n.plane));
            node.links = n.links.map(|l| (Some(l) != none && (l as usize) < count).then_some(l));
            node
        })
        .collect();
    let header = SceneAiMap {
        bbox: nodes_bbox(&nodes),
        params: AiParams { patch_size: graph.header.cell_size, ..AiParams::default() },
        ..SceneAiMap::default()
    };
    AiMapPart::from_nodes(header, &nodes, None)
}

impl BaseSource for LibraryLevels {
    fn level(&self, name: &str, settings: Option<&(dyn Settings + Sync)>) -> Result<Arc<SceneState>, String> {
        let editor = self.editor.upgrade().ok_or("the editor is closing")?;
        let root = GameLibrary::game_root(&editor);
        if root.is_empty() {
            return Err("no game is linked (`game.link`)".to_owned());
        }
        let files = GameLibrary::of(&editor).files_blocking(&editor, &root)?;
        let level = name.to_ascii_lowercase();
        if !files.exists(&format!("levels/{level}/level")) {
            return Err(format!(
                "the game at {root} has no level of that name (`asset.find kind=level` lists its levels)"
            ));
        }
        let stamps = Stamps::of(&files);
        let spawn_stamp =
            files.get("spawns/all.spawn").map_or_else(|| "missing".to_owned(), |f| stamps.of_file(f));
        // entities read without the game's configs are another scene than with them
        let stamp = format!(
            "{}|{}",
            level_stamp(&files, &stamps, &level, &spawn_stamp),
            if settings.is_some() { "decoded" } else { "raw" }
        );
        let held = |me: &Self| {
            me.made.lock().get(&level).filter(|(made_from, _)| *made_from == stamp).map(|(_, s)| s.clone())
        };
        if let Some(state) = held(self) {
            return Ok(state);
        }
        let turn = self.making.lock().entry(level.clone()).or_default().clone();
        let _making = turn.lock();
        // whoever had the turn before may have made it
        if let Some(state) = held(self) {
            return Ok(state);
        }
        let spawn = Spawn::read(&files).ok();
        let (mut state, _) = scene_of(&files, &level, spawn.as_ref(), settings)?;
        if let Ok(bytes) = files.read(&format!("levels/{level}/level.ai"))
            && let Ok(graph) = LevelGraph::read(&bytes)
            && !graph.nodes.is_empty()
        {
            state.bulk.ai_map = Some(Arc::new(ai_layer(&graph)));
        }
        let state = Arc::new(gz_scene::base::marked(state));
        self.made.lock().insert(level, (stamp, state.clone()));
        Ok(state)
    }

    fn names(&self) -> Vec<SmolStr> {
        let Some(editor) = self.editor.upgrade() else { return Vec::new() };
        let root = GameLibrary::game_root(&editor);
        if root.is_empty() {
            return Vec::new();
        }
        GameLibrary::of(&editor)
            .files_blocking(&editor, &root)
            .map(|files| level_names(&files).into_iter().map(SmolStr::new).collect())
            .unwrap_or_default()
    }
}

/// Registers the game's levels as what a scene may lie over.
pub fn install(editor: &Arc<Editor>) {
    BaseLevels::of(editor).set(Arc::new(LibraryLevels {
        editor: Arc::downgrade(editor),
        made: Mutex::new(FxHashMap::default()),
        making: Mutex::new(FxHashMap::default()),
    }));
}
