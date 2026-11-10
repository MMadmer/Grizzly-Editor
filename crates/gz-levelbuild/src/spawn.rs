//! The spawn file of the scene (`CLevelSpawnConstructor` + `CGameSpawnConstructor` of the level
//! editor: one level, the editor's game graph embedded).

use std::collections::BTreeMap;

use glam::Vec3;
use gz_entities::model::alife_flags;
use gz_entities::{Class, Entity, Shape};
use gz_fmt_spawn::{Bytes, GameGraph, Guid, LevelPoint, PatrolLink, PatrolPath, PatrolPoint, SpawnFile, SpawnHeader, SpawnVertex, XRAI_VERSION_PRIQUEL};

use crate::nav::{INVALID, LevelNav};
use crate::{BuildError, Result};

/// `y_shift_correction`: a spawn is looked up a little above where it stands.
const Y_SHIFT: f32 = 0.15;

/// A spawn element as the scene has it, its entity made ready for export (`PreExportSpawn`):
/// name, position, rotation and the attached shape's geometry.
#[derive(Clone, Debug)]
pub struct Element {
    pub entity: Entity,
}

impl Element {
    /// The entity with the scene's name, transform and shapes.
    pub fn new(mut entity: Entity, name: &str, position: Vec3, rotation: Vec3, shapes: Option<&[Shape]>) -> Self {
        let _ = entity.set_name(name);
        let _ = entity.set_position(position);
        let _ = entity.set_angle(rotation);
        if let Some(s) = shapes {
            let _ = entity.set_shapes(s);
        }
        Self { entity }
    }
}

/// A way object of the scene.
#[derive(Clone, Debug, PartialEq)]
pub struct Way {
    pub name: String,
    /// Name, position, flags.
    pub points: Vec<(String, Vec3, u32)>,
    /// From, to, probability, in the order of the points' links.
    pub links: Vec<(u32, u32, f32)>,
}

/// What the spawn build made.
#[derive(Clone, Debug)]
pub struct SpawnBuild {
    pub bytes: Vec<u8>,
    pub log: Vec<String>,
}

fn class_of(e: &Entity) -> Option<Class> {
    e.class_info().map(|c| c.class)
}

/// The offset of `m_tSpawnID` in a spawn packet (`CSE_Abstract::Spawn_Write`).
fn spawn_id_offset(p: &[u8]) -> Option<usize> {
    let mut at = 2usize;
    for _ in 0..2 {
        at += p.get(at..)?.iter().position(|b| *b == 0)? + 1;
    }
    // game id, rp, position, angle, respawn, id, parent, phantom
    at += 2 + 24 + 2 + 6;
    let flags = u16::from_le_bytes([*p.get(at)?, *p.get(at + 1)?]);
    at += 2;
    let v = if flags & (1 << 5) != 0 {
        let v = u16::from_le_bytes([*p.get(at)?, *p.get(at + 1)?]);
        at += 2;
        v
    } else {
        0
    };
    if v <= 79 {
        return None;
    }
    if v > 120 {
        at += 2;
    }
    if v > 69 {
        at += 2;
    }
    if v > 70 {
        let n = if v > 93 {
            let n = usize::from(u16::from_le_bytes([*p.get(at)?, *p.get(at + 1)?]));
            at += 2;
            n
        } else {
            let n = usize::from(*p.get(at)?);
            at += 1;
            n
        };
        at += n;
    }
    (at + 2 <= p.len()).then_some(at)
}

/// The game vertex range of level 0 and its first vertex.
fn level_vertices(graph: &GameGraph) -> Option<u16> {
    graph.vertices.iter().position(|v| v.level_id == 0).map(|i| i as u16)
}

/// `CGameSpawnConstructor::build`: every spawn element on the level vertex under it and the
/// game vertex that owns that, the zones' artefact points, the patrol paths, the header, the
/// game graph. `level` names the level (`m_FNLevelPath`).
// oracle: the artefact points of a zone come in the walk's order here; the C++ shuffles them
// with `rand`. A zone's `m_artefact_position_offset` and a level changer's destination stay as
// the scene holds them: `gz-entities` does not offer them for writing yet.
pub fn build_spawn(level: &str, nav: &LevelNav, graph: &GameGraph, elements: Vec<Element>, ways: &[Way], guid: Guid) -> Result<SpawnBuild> {
    let mut log = vec!["Start build spawn".to_owned(), format!("level  0 {level}"), "Loading spawns ...".to_owned()];
    let cross = graph.cross_tables.first().ok_or_else(|| BuildError::new("! The game graph has no cross table."))?;
    let mut spawns: Vec<Entity> = Vec::new();
    let mut story: BTreeMap<u32, String> = BTreeMap::new();
    let mut actors = 0usize;
    for el in elements {
        let e = el.entity;
        let Some(class) = class_of(&e) else { continue };
        if class == Class::GraphPoint || !class.is_alife() {
            continue;
        }
        if class == Class::CreatureActor {
            actors += 1;
            if actors > 1 {
                return Err(BuildError::new("! Too many actors on the level"));
            }
        }
        if let Some(id) = e.story_id() {
            if let Some(other) = story.get(&id) {
                log.push(format!("Object {}, story id {id}", e.name()));
                log.push(format!("Object {other}, story id {id}"));
                return Err(BuildError::new(format!("There are several objects which has the same unique story ID, level {level}")));
            }
            story.insert(id, e.name().to_owned());
        }
        spawns.push(e);
    }
    if spawns.is_empty() {
        return Err(BuildError::new("! There are no spawn-points!"));
    }

    log.push("Correct objects ...".to_owned());
    let Some(first) = level_vertices(graph) else {
        return Err(BuildError::new(format!("! There are no graph vertices in the game graph for the level '{level}' !")));
    };
    let first_lv = graph.vertices.get(usize::from(first)).map_or(INVALID, |v| v.level_vertex);
    let mut artefact_points: Vec<LevelPoint> = Vec::new();
    for e in &mut spawns {
        let used_ai = e.object().is_some_and(|o| o.alife().flags & alife_flags::USED_AI_LOCATIONS != 0);
        if !used_ai {
            let _ = e.set_vertices(first, first_lv, 0.0);
            continue;
        }
        let mut position = e.position();
        position.y += Y_SHIFT;
        let node = nav.vertex(INVALID, position);
        if !nav.inside(node, position) {
            let moved = nav.vertex_position(node);
            log.push(format!(
                "[{level}][{}][{}] : position changed from [{:.6}][{:.6}][{:.6}] -> [{:.6}][{:.6}][{:.6}]",
                e.section(),
                e.name(),
                position.x,
                position.y,
                position.z,
                moved.x,
                moved.y,
                moved.z
            ));
            let _ = e.set_position(moved);
        }
        let Some((gv, dist)) = cross.cell(node) else {
            let p = e.position();
            return Err(BuildError::new(format!(
                "Can't find a corresponding GRAPH VERTEX for the spawn-point {}\nSpawn node  : {node}\nSpawn point : [{:7.2}][{:7.2}][{:7.2}]",
                e.name(),
                p.x,
                p.y,
                p.z
            )));
        };
        let _ = e.set_vertices(gv, node, dist);
    }

    log.push("Generate artefact spawn positions ...".to_owned());
    for e in &mut spawns {
        let Some(zone) = class_of(e).filter(|c| c.is_anomalous_zone()).and_then(|_| e.object().and_then(|o| o.zone()).cloned()) else { continue };
        let position = e.position();
        let current = e.level_vertex_id().unwrap_or(INVALID);
        let mut node = nav.vertex(current, position);
        if !nav.valid_position(position) || !nav.inside(node, position) {
            node = nav.vertex(INVALID, position);
        }
        if let Some((gv, dist)) = cross.cell(node) {
            let _ = e.set_vertices(gv, node, dist);
        }
        let r2 = zone.offline_interactive_radius * zone.offline_interactive_radius;
        let around: Vec<u32> = nav.flood(node, zone.offline_interactive_radius).into_iter().filter(|&n| nav.vertex_position(n).distance_squared(position) <= r2).collect();
        for n in around.into_iter().take(usize::from(zone.artefact_spawn_count)) {
            artefact_points.push(LevelPoint { point: nav.vertex_position(n), level_vertex: n, distance: cross.cell(n).map_or(0.0, |c| c.1) });
        }
    }
    for e in &mut spawns {
        if class_of(e) != Some(Class::LevelChanger) {
            continue;
        }
        let mut position = e.position();
        position.y += Y_SHIFT;
        let node = nav.vertex(INVALID, position);
        if let Some((gv, dist)) = cross.cell(node) {
            let _ = e.set_vertices(gv, node, dist);
        }
    }
    if actors == 0 {
        return Err(BuildError::new("! There is no ACTOR spawn point!"));
    }
    log.push(format!("Actor is on the level {level}"));

    // the patrol paths of the way objects, by name
    let mut by_name: BTreeMap<&str, &Way> = BTreeMap::new();
    for w in ways {
        if by_name.insert(w.name.as_str(), w).is_some() {
            return Err(BuildError::new(format!("Duplicated patrol path found {}", w.name)));
        }
    }
    let patrols: Vec<PatrolPath> = by_name.values().map(|w| patrol(nav, cross, w)).collect();

    let mut file = SpawnFile::new(
        SpawnHeader { version: XRAI_VERSION_PRIQUEL, guid, graph_guid: graph.guid, spawn_count: spawns.len() as u32, level_count: 1 },
        Some(graph.clone()),
    );
    for (i, e) in spawns.iter().enumerate() {
        let id = u16::try_from(i).map_err(|_| BuildError::new("too many spawn objects"))?;
        let mut packet = e.to_spawn_packet();
        if let Some(at) = spawn_id_offset(&packet) {
            packet[at..at + 2].copy_from_slice(&id.to_le_bytes());
        }
        let update = e.update_packet().unwrap_or_default();
        file.vertices.push(SpawnVertex { id, spawn: Bytes::from_vec(packet), update: Bytes::from_vec(update) });
    }
    file.declared_vertices = file.vertices.len() as u32;
    file.artefact_points = artefact_points;
    file.declared_patrols = patrols.len() as u32;
    file.patrols = patrols;
    log.push("Spawn build completed".to_owned());
    Ok(SpawnBuild { bytes: file.to_bytes(), log })
}

/// `CPatrolPath::load_editor`: each point on the level vertex under it, moved onto it when it
/// stands outside, with the game vertex of that.
fn patrol(nav: &LevelNav, cross: &gz_fmt_spawn::CrossTable, w: &Way) -> PatrolPath {
    let points = w
        .points
        .iter()
        .enumerate()
        .map(|(i, (name, pos, flags))| {
            let mut position = *pos;
            let level_vertex = if nav.valid_position(position) { nav.vertex_id(position + Vec3::new(0.0, Y_SHIFT, 0.0)) } else { INVALID };
            let mut game_vertex = u16::MAX;
            if nav.valid_position(position) && nav.valid_id(level_vertex) {
                if !nav.inside(level_vertex, position) {
                    position = nav.vertex_position(level_vertex);
                }
                game_vertex = cross.cell(level_vertex).map_or(u16::MAX, |c| c.0);
            }
            (i as u32, PatrolPoint { name: name.as_str().into(), position, flags: *flags, level_vertex, game_vertex })
        })
        .collect::<Vec<_>>();
    let mut links: Vec<PatrolLink> = Vec::new();
    for &(from, to, p) in &w.links {
        match links.iter_mut().find(|l| l.from == from) {
            Some(l) => l.to.push((to, p)),
            None => links.push(PatrolLink { from, to: vec![(to, p)] }),
        }
    }
    PatrolPath { name: w.name.as_str().into(), declared_points: points.len() as u32, points, links, has_links_chunk: true }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spawn_id_is_found_in_a_packet() {
        let mut p = vec![1u8, 0];
        p.extend_from_slice(b"stalker\0guard\0");
        p.extend_from_slice(&[0, 0xfe]);
        p.extend_from_slice(&[0u8; 24]);
        p.extend_from_slice(&[0, 0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        p.extend_from_slice(&(1u16 << 5).to_le_bytes());
        p.extend_from_slice(&128u16.to_le_bytes());
        p.extend_from_slice(&[0, 0]); // game type
        p.extend_from_slice(&[6, 0]); // script version
        p.extend_from_slice(&[0, 0]); // client data size
        p.extend_from_slice(&[0x34, 0x12]);
        let at = spawn_id_offset(&p).unwrap();
        assert_eq!(&p[at..at + 2], &[0x34, 0x12]);
    }
}
