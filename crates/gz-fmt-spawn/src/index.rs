//! What the editor's world reader asks of the linked game's `all.spawn`: the levels, every
//! entity with its level, and every patrol path name. Built in parallel, one pass.

use glam::Vec3;
use rayon::prelude::*;
use smol_str::SmolStr;

use crate::peek::peek_packet;
use crate::spawn::SpawnFile;

#[derive(Clone, Debug, PartialEq)]
pub struct IndexLevel {
    pub id: u8,
    pub name: SmolStr,
    pub section: SmolStr,
    pub offset: Vec3,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IndexEntity {
    /// The vertex of the spawn graph.
    pub spawn_id: u16,
    pub section: SmolStr,
    pub name: SmolStr,
    pub position: Vec3,
    pub game_vertex: Option<u16>,
    pub level_vertex: Option<u32>,
    /// The game vertex's level; `None` without a game graph or for a vertex it lacks.
    pub level: Option<u8>,
    /// The packet's head did not read (the entity is listed with what is known).
    pub damaged: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpawnIndex {
    pub levels: Vec<IndexLevel>,
    pub entities: Vec<IndexEntity>,
    pub patrol_names: Vec<SmolStr>,
}

impl SpawnIndex {
    #[must_use]
    pub fn build(file: &SpawnFile) -> Self {
        let graph = file.game_graph.as_ref();
        let levels = graph
            .map(|g| {
                g.levels
                    .iter()
                    .map(|l| IndexLevel {
                        id: l.id,
                        name: l.name.clone(),
                        section: l.section.clone(),
                        offset: l.offset,
                    })
                    .collect()
            })
            .unwrap_or_default();
        let entities = file
            .vertices
            .par_iter()
            .map(|v| match peek_packet(&v.spawn) {
                Ok(h) => IndexEntity {
                    spawn_id: v.id,
                    level: h
                        .game_vertex
                        .and_then(|gv| graph?.vertices.get(usize::from(gv)).map(|x| x.level_id)),
                    section: h.section,
                    name: h.name,
                    position: h.position,
                    game_vertex: h.game_vertex,
                    level_vertex: h.level_vertex,
                    damaged: false,
                },
                Err(_) => IndexEntity {
                    spawn_id: v.id,
                    section: SmolStr::default(),
                    name: SmolStr::default(),
                    position: Vec3::ZERO,
                    game_vertex: None,
                    level_vertex: None,
                    level: None,
                    damaged: true,
                },
            })
            .collect();
        let patrol_names = file.patrols.iter().map(|p| p.name.clone()).collect();
        Self { levels, entities, patrol_names }
    }

    #[must_use]
    pub fn level_name(&self, id: u8) -> Option<&str> {
        self.levels.iter().find(|l| l.id == id).map(|l| l.name.as_str())
    }

    /// The entities on one level.
    pub fn on_level(&self, id: u8) -> impl Iterator<Item = &IndexEntity> {
        self.entities.iter().filter(move |e| e.level == Some(id))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::bytes::Bytes;
    use crate::graph::tests::sample_graph;
    use crate::patrol::tests::sample_path;
    use crate::peek::tests::packet;
    use crate::spawn::{SpawnHeader, SpawnVertex};

    #[test]
    fn entities_find_their_level_through_the_graph() {
        let mut f = SpawnFile::new(
            SpawnHeader { version: 10, guid: [0; 16], graph_guid: [7; 16], spawn_count: 3, level_count: 1 },
            Some(sample_graph()),
        );
        f.vertices = vec![
            SpawnVertex {
                id: 0,
                spawn: Bytes::from_vec(packet("stalker", "a", 1, 5)),
                update: Bytes::default(),
            },
            SpawnVertex {
                id: 1,
                spawn: Bytes::from_vec(packet("medkit", "b", 9, 5)),
                update: Bytes::default(),
            },
            SpawnVertex { id: 2, spawn: Bytes::from_vec(vec![1, 0, b'x']), update: Bytes::default() },
        ];
        f.declared_vertices = 3;
        f.patrols = vec![sample_path("esc_walk")];
        f.declared_patrols = 1;
        let file = SpawnFile::parse(Arc::from(f.to_bytes())).unwrap();
        let idx = SpawnIndex::build(&file);
        assert_eq!(idx.level_name(0), Some("l01_escape"));
        assert_eq!(idx.entities[0].level, Some(0));
        assert_eq!(idx.entities[1].level, None);
        assert!(idx.entities[2].damaged);
        assert_eq!(idx.on_level(0).count(), 1);
        assert_eq!(idx.patrol_names, vec![SmolStr::new("esc_walk")]);
    }
}
