//! The spawn file of the scene (`CLevelSpawnConstructor` + `CGameSpawnConstructor` of the level
//! editor: one level, the editor's game graph embedded).

use std::collections::BTreeMap;

use glam::Vec3;
use gz_entities::model::alife_flags;
use gz_entities::{Class, Entity, Shape};
use gz_fmt_object::Mat4;
use gz_fmt_spawn::{
    Bytes, GameGraph, Guid, LevelPoint, PatrolLink, PatrolPath, PatrolPoint, SpawnFile, SpawnHeader,
    SpawnVertex, XRAI_VERSION_PRIQUEL,
};

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
    pub fn new(
        mut entity: Entity,
        name: &str,
        position: Vec3,
        rotation: Vec3,
        shapes: Option<&[Shape]>,
    ) -> Self {
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

/// The first game vertex of a level.
fn level_vertices(graph: &GameGraph, level_id: u8) -> Option<u16> {
    graph.vertices.iter().position(|v| v.level_id == level_id).map(|i| i as u16)
}

/// `CGameSpawnConstructor::build`: every spawn element on the level vertex under it and the
/// game vertex that owns that, the zones' artefact points, the patrol paths, the header, the
/// game graph. `level` names the level (`m_FNLevelPath`).
// oracle: the artefact points of a zone come in the walk's order here; the C++ shuffles them
// with `rand`.
/// `verify_restrictors` is Play's "Verify Space Restrictors": every space restrictor that asks
/// for it must lie on the AI map (`verify_space_restrictors`; the editor never runs the separator
/// part of it).
pub fn build_spawn(
    level: &str,
    nav: &LevelNav,
    graph: &GameGraph,
    elements: Vec<Element>,
    ways: &[Way],
    guid: Guid,
    verify_restrictors: bool,
) -> Result<SpawnBuild> {
    let mut log =
        vec!["Start build spawn".to_owned(), format!("level  0 {level}"), "Loading spawns ...".to_owned()];
    // `game_graph().header().level(level_name)`: Play's graph holds the linked game's levels too
    let level_id = graph
        .levels
        .iter()
        .find(|l| l.name.eq_ignore_ascii_case(level))
        .map(|l| l.id)
        .ok_or_else(|| BuildError::new(format!("! The game graph has no level {level}.")))?;
    let cross =
        graph.cross_table(level_id).ok_or_else(|| BuildError::new("! The game graph has no cross table."))?;
    let mut spawns: Vec<Entity> = Vec::new();
    let mut story: BTreeMap<u32, String> = BTreeMap::new();
    let mut actors = 0usize;
    // the graph points a level changer of this level may name: (name, position, direction)
    let mut graph_points: Vec<(String, Vec3, Vec3)> = Vec::new();
    for el in elements {
        let mut e = el.entity;
        let Some(class) = class_of(&e) else { continue };
        if class == Class::GraphPoint {
            graph_points.push((e.name().to_owned(), e.position(), e.angle()));
            continue;
        }
        if !class.is_alife() {
            continue;
        }
        if class == Class::CreatureActor {
            actors += 1;
            if actors > 1 {
                return Err(BuildError::new("! Too many actors on the level"));
            }
            // oracle: the C++ keeps the mark the spawn point got from `$player`; the game makes
            // no level current without it (`CALifeGraphRegistry::update`), so the one actor
            // carries it here whatever the scene saved
            if !e.is_player() && e.set_player().is_err() {
                return Err(BuildError::new(format!("! The actor {} cannot be made the player.", e.name())));
            }
        }
        if let Some(id) = e.story_id() {
            if let Some(other) = story.get(&id) {
                log.push(format!("Object {}, story id {id}", e.name()));
                log.push(format!("Object {other}, story id {id}"));
                return Err(BuildError::new(format!(
                    "There are several objects which has the same unique story ID, level {level}"
                )));
            }
            story.insert(id, e.name().to_owned());
        }
        spawns.push(e);
    }
    if spawns.is_empty() {
        return Err(BuildError::new("! There are no spawn-points!"));
    }

    log.push("Correct objects ...".to_owned());
    let Some(first) = level_vertices(graph, level_id) else {
        return Err(BuildError::new(format!(
            "! There are no graph vertices in the game graph for the level '{level}' !"
        )));
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
        let Some(zone) = class_of(e)
            .filter(|c| c.is_anomalous_zone())
            .and_then(|_| e.object().and_then(|o| o.zone()).cloned())
        else {
            continue;
        };
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
        let around: Vec<u32> = nav
            .flood(node, zone.offline_interactive_radius)
            .into_iter()
            .filter(|&n| nav.vertex_position(n).distance_squared(position) <= r2)
            .collect();
        // the zone's points start where the list stands now
        let _ = e.set_artefact_offset(artefact_points.len() as u32);
        for n in around.into_iter().take(usize::from(zone.artefact_spawn_count)) {
            artefact_points.push(LevelPoint {
                point: nav.vertex_position(n),
                level_vertex: n,
                distance: cross.cell(n).map_or(0.0, |c| c.1),
            });
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
        fill_level_changer(e, level, level_id, graph, &graph_points, &mut log)?;
    }
    if actors == 0 {
        return Err(BuildError::new("! There is no ACTOR spawn point!"));
    }
    if verify_restrictors && !verify_space_restrictors(level, nav, &spawns, &mut log) {
        log.push("! Failed build spawn!".to_owned());
        return Err(BuildError::new(format!(
            "! Failed build spawn! 
See log.
{}",
            log.join(
                "
"
            )
        )));
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
        SpawnHeader {
            version: XRAI_VERSION_PRIQUEL,
            guid,
            graph_guid: graph.guid,
            spawn_count: spawns.len() as u32,
            level_count: 1,
        },
        Some(graph.clone()),
    );
    for (i, e) in spawns.iter().enumerate() {
        let id = u16::try_from(i).map_err(|_| BuildError::new("too many spawn objects"))?;
        let mut packet = e.to_spawn_packet();
        if let Some(at) = spawn_id_offset(&packet) {
            packet[at..at + 2].copy_from_slice(&id.to_le_bytes());
        }
        let update = e.update_packet().unwrap_or_default();
        file.vertices.push(SpawnVertex {
            id,
            spawn: Bytes::from_vec(packet),
            update: Bytes::from_vec(update),
        });
    }
    file.declared_vertices = file.vertices.len() as u32;
    file.artefact_points = artefact_points;
    file.declared_patrols = patrols.len() as u32;
    file.patrols = patrols;
    log.push("Spawn build completed".to_owned());
    Ok(SpawnBuild { bytes: file.to_bytes(), log })
}

/// A space restrictor laid over the AI map (`CSpaceRestrictorWrapper`).
struct Restriction<'a> {
    xform: Mat4,
    position: Vec3,
    shapes: &'a [Shape],
}

/// `Fplane::build(p0, p1, p2)` and `classify`: how far `p` is in front of the plane.
fn plane_classify(p0: Vec3, p1: Vec3, p2: Vec3, p: Vec3) -> f32 {
    let n = (p1 - p0).cross(p2 - p0).normalize_or_zero();
    n.dot(p) - n.dot(p0)
}

const BOX_CORNERS: [Vec3; 8] = [
    Vec3::new(-0.5, -0.5, -0.5),
    Vec3::new(-0.5, -0.5, 0.5),
    Vec3::new(-0.5, 0.5, 0.5),
    Vec3::new(-0.5, 0.5, -0.5),
    Vec3::new(0.5, 0.5, 0.5),
    Vec3::new(0.5, 0.5, -0.5),
    Vec3::new(0.5, -0.5, 0.5),
    Vec3::new(0.5, -0.5, -0.5),
];

fn box_matrix(i: Vec3, j: Vec3, k: Vec3, c: Vec3) -> Mat4 {
    Mat4([[i.x, i.y, i.z, 0.0], [j.x, j.y, j.z, 0.0], [k.x, k.y, k.z, 0.0], [c.x, c.y, c.z, 1.0]])
}

impl Restriction<'_> {
    /// `inside(position, radius)`: a sphere of `radius` at `p` touches one of the shapes.
    fn touches(&self, p: Vec3, radius: f32) -> bool {
        self.shapes.iter().any(|shape| match *shape {
            Shape::Sphere { center, radius: r } => {
                let c = self.xform.transform_tiny(center);
                let sum = radius + r;
                p.distance_squared(c) < sum * sum
            }
            Shape::Box { i, j, k, c } => {
                let q = Mat4::mul_43(&self.xform, &box_matrix(i, j, k, c));
                let v: Vec<Vec3> = BOX_CORNERS.iter().map(|&corner| q.transform_tiny(corner)).collect();
                let faces = [(0, 3, 5), (1, 2, 3), (6, 5, 4), (4, 2, 1), (3, 2, 4), (1, 0, 6)];
                faces.iter().all(|&(a, b, c)| plane_classify(v[a], v[b], v[c], p) <= radius)
            }
            Shape::Other(_) => false,
        })
    }

    /// `inside(level_vertex_id, partially_inside)`: the vertex's four corners and its centre.
    fn covers(&self, nav: &LevelNav, id: u32, partially: bool) -> bool {
        let p = nav.vertex_position(id);
        let offset = nav.cell_size() * 0.5 - gz_scene::math::EPS_L;
        let at = |x: f32, z: f32| Vec3::new(x, nav.plane_y(id, x, z), z);
        let points = [
            at(p.x + offset, p.z + offset),
            at(p.x + offset, p.z - offset),
            at(p.x - offset, p.z + offset),
            at(p.x - offset, p.z - offset),
            p,
        ];
        if partially {
            points.iter().any(|&q| self.touches(q, gz_scene::math::EPS_L))
        } else {
            points.iter().all(|&q| self.touches(q, gz_scene::math::EPS_L))
        }
    }

    /// `fill_shape` over every shape, `build_border`: the vertices the restrictor covers in
    /// part only. An empty border means it does not lie on the AI map.
    fn has_border(&self, nav: &LevelNav) -> bool {
        for shape in self.shapes {
            let (start, dest) = match *shape {
                Shape::Sphere { center, radius } => {
                    let o = Vec3::new(radius, 0.0, radius);
                    (center - o + self.position, center + o + self.position)
                }
                Shape::Box { i, j, k, c } => {
                    let q = Mat4::mul_43(&self.xform, &box_matrix(i, j, k, c));
                    let pts: Vec<Vec3> = BOX_CORNERS.iter().map(|&corner| q.transform_tiny(corner)).collect();
                    (
                        pts.iter().copied().fold(Vec3::splat(f32::MAX), Vec3::min),
                        pts.iter().copied().fold(Vec3::splat(f32::MIN), Vec3::max),
                    )
                }
                Shape::Other(_) => continue,
            };
            let found = (0..nav.count()).any(|id| {
                let p = nav.vertex_position(id);
                p.x >= start.x
                    && p.x <= dest.x
                    && p.z >= start.z
                    && p.z <= dest.z
                    && self.covers(nav, id, true)
                    && !self.covers(nav, id, false)
            });
            if found {
                return true;
            }
        }
        false
    }
}

/// `CLevelSpawnConstructor::verify_space_restrictors` without the separator check (the editor
/// builds with `no_separator_check`): every restrictor of a type that restricts and that asks
/// for the check must have a border on the AI map.
fn verify_space_restrictors(level: &str, nav: &LevelNav, spawns: &[Entity], log: &mut Vec<String>) -> bool {
    log.push(format!("Level [{level}] : searching for AI map separators space restrictors"));
    let mut ok = true;
    for e in spawns {
        let Some(obj) = e.object() else { continue };
        let Some(r) = obj.restrictor() else { continue };
        if r.restrictor_type == 0 || obj.alife().flags & alife_flags::CHECK_FOR_SEPARATOR == 0 {
            continue;
        }
        let a = e.angle();
        let mut xform = Mat4::hpb(a.y, a.x, a.z);
        let p = e.position();
        xform.0[3] = [p.x, p.y, p.z, 1.0];
        let wrap = Restriction { xform, position: p, shapes: e.shapes() };
        if !wrap.has_border(nav) {
            log.push(format!("! space restrictor has no border {}", e.name()));
            ok = false;
        }
    }
    log.push(format!("Level [{level}] : no separators found"));
    ok
}

/// `fill_level_changers`: a level changer that leads into this very level goes to the graph point
/// it names - its game vertex (the vertex of this level standing on the point), the vertex's
/// level vertex, the point's position and direction. One that leads elsewhere keeps what it has.
fn fill_level_changer(
    e: &mut Entity,
    level: &str,
    level_id: u8,
    graph: &GameGraph,
    points: &[(String, Vec3, Vec3)],
    log: &mut Vec<String>,
) -> Result<()> {
    let Some(dest) = e.level_changer() else { return Ok(()) };
    if !dest.level.eq_ignore_ascii_case(level) {
        return Ok(());
    }
    let Some((_, position, angles)) = points.iter().find(|(n, _, _)| *n == dest.point.as_str()) else {
        log.push(format!("Graph point {} not found (level changer {})", dest.point, e.name()));
        return Ok(());
    };
    let similar = |a: Vec3, b: Vec3| {
        (a.x - b.x).abs() <= 0.001 && (a.y - b.y).abs() <= 0.001 && (a.z - b.z).abs() <= 0.001
    };
    let found = graph
        .vertices
        .iter()
        .enumerate()
        .find(|(_, v)| v.level_id == level_id && similar(v.local_point, *position));
    let Some((index, vertex)) = found else {
        return Err(BuildError::new(format!(
            "Cannot find a correspndance between graph and graph points from level editor! Rebuild graph for the level {}",
            dest.level
        )));
    };
    let _ = e.set_level_changer_destination(index as u16, vertex.level_vertex, *position, *angles);
    Ok(())
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
            let level_vertex = if nav.valid_position(position) {
                nav.vertex_id(position + Vec3::new(0.0, Y_SHIFT, 0.0))
            } else {
                INVALID
            };
            let mut game_vertex = u16::MAX;
            if nav.valid_position(position) && nav.valid_id(level_vertex) {
                if !nav.inside(level_vertex, position) {
                    position = nav.vertex_position(level_vertex);
                }
                game_vertex = cross.cell(level_vertex).map_or(u16::MAX, |c| c.0);
            }
            (
                i as u32,
                PatrolPoint {
                    name: name.as_str().into(),
                    position,
                    flags: *flags,
                    level_vertex,
                    game_vertex,
                },
            )
        })
        .collect::<Vec<_>>();
    let mut links: Vec<PatrolLink> = Vec::new();
    for &(from, to, p) in &w.links {
        match links.iter_mut().find(|l| l.from == from) {
            Some(l) => l.to.push((to, p)),
            None => links.push(PatrolLink { from, to: vec![(to, p)] }),
        }
    }
    PatrolPath {
        name: w.name.as_str().into(),
        declared_points: points.len() as u32,
        points,
        links,
        has_links_chunk: true,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::graph::{GraphPoint, build_game_graph, tests::grid};
    use gz_entities::MapSettings;
    use gz_fmt_level::LevelGraph;

    fn settings() -> MapSettings {
        let creature = [
            ("ef_creature_type", "1"),
            ("going_speed", "1.5"),
            ("terrain", "stalker_terrain"),
            ("MaxHealthValue", "200"),
            ("eye_range", "100"),
            ("visual", "actors\\x\\x.ogf"),
        ];
        let item =
            [("class", "II_MEDKI"), ("inv_weight", "0.5"), ("cost", "100"), ("visual", "Dynamics\\Item.ogf")];
        let actor: Vec<(&str, &str)> = creature
            .iter()
            .copied()
            .chain([("class", "S_ACTOR"), ("max_item_mass", "50"), ("$player", "on")])
            .collect();
        MapSettings::new()
            .section("script", &[("current_server_entity_version", "12")])
            .section("graph_point", &[("class", "AI_GRAPH")])
            .section("actor", &actor)
            .section("medkit", &item)
    }

    /// What the game's loader relies on in a spawn file (`CALifeSpawnRegistry::load`,
    /// `CGameGraph`, `AISpaceBase::Load` / `Validate`, `CALifeGraphRegistry::setup_current_level`),
    /// read back with the repository's reader.
    fn check_spawn_file(bytes: &[u8], level: &str, level_ai: &[u8], st: &MapSettings) -> Vec<String> {
        let mut bad = Vec::new();
        let lg = LevelGraph::read(level_ai).unwrap();
        let nodes = lg.nodes.len() as u32;
        let file = SpawnFile::parse(Arc::from(bytes)).unwrap();
        let h = &file.header;
        let Some(g) = &file.game_graph else { return vec!["no game graph (chunk 4)".into()] };
        if h.version != XRAI_VERSION_PRIQUEL {
            bad.push(format!("header version {}", h.version));
        }
        if h.spawn_count as usize != file.vertices.len()
            || file.declared_vertices as usize != file.vertices.len()
        {
            bad.push(format!(
                "spawn counts {} / {} / {}",
                h.spawn_count,
                file.declared_vertices,
                file.vertices.len()
            ));
        }
        // the levels whose objects the file holds (`m_level_spawns`): Play's are one of the graph's
        if h.level_count == 0 || h.level_count as usize > g.levels.len() {
            bad.push(format!("level count {} for {} levels", h.level_count, g.levels.len()));
        }
        if h.graph_guid != g.guid {
            bad.push("the header's graph guid is not the graph's".into());
        }
        if usize::from(g.vertex_count) != g.vertices.len() || g.edge_count as usize != g.edges.len() {
            bad.push("the graph's counts are not its records".into());
        }
        let Some(lvl) = g.levels.iter().find(|l| l.name == level) else {
            bad.push(format!("the level table has no level {level}"));
            return bad;
        };
        if lvl.guid != lg.header.guid {
            bad.push("the level's guid is not the AI map's".into());
        }
        let Some(cross) = g.cross_table(lvl.id) else {
            bad.push("the level has no cross table".into());
            return bad;
        };
        if g.cross_tables.len() != g.levels.len() {
            bad.push("one cross table per level".into());
        }
        if cross.level_guid != lg.header.guid || cross.game_guid != g.guid {
            bad.push("the cross table's guids".into());
        }
        if cross.level_vertex_count != nodes
            || cross.cell_count() != nodes as usize
            || cross.game_vertex_count != u32::from(g.vertex_count)
        {
            bad.push("the cross table's counts".into());
        }
        for (i, v) in g.vertices.iter().enumerate() {
            if g.level(v.level_id).is_none() {
                bad.push(format!("graph vertex {i}: level id {} is not in the level table", v.level_id));
            }
            if g.edges_of(i as u16).len() != usize::from(v.edge_count) {
                bad.push(format!("graph vertex {i}: edges"));
            }
            if v.level_id == lvl.id && cross.cell(v.level_vertex).map(|c| c.0) != Some(i as u16) {
                bad.push(format!("graph vertex {i}: the cross table gives its level vertex to another"));
            }
        }
        let mut players = 0;
        for (i, v) in file.vertices.iter().enumerate() {
            if usize::from(v.id) != i {
                bad.push(format!("spawn {i} has id {}", v.id));
            }
            let e = Entity::from_packets(&v.spawn, &v.update, st).unwrap();
            if e.header().spawn_id != v.id {
                bad.push(format!("spawn {i}: the packet's spawn id {}", e.header().spawn_id));
            }
            let gv = e.game_vertex_id().unwrap_or(u16::MAX);
            let lv = e.level_vertex_id().unwrap_or(u32::MAX);
            if gv >= g.vertex_count || g.level_of_vertex(gv).map(|l| l.id) != Some(lvl.id) {
                bad.push(format!("spawn {i}: game vertex {gv} is not a vertex of the level"));
            }
            if lv >= nodes || cross.cell(lv).map(|c| c.0) != Some(gv) {
                bad.push(format!("spawn {i}: level vertex {lv} does not lead to game vertex {gv}"));
            }
            if class_of(&e) == Some(Class::CreatureActor) {
                let direct = e.object().is_some_and(|o| o.alife().direct_control);
                if !e.is_player() || !direct {
                    bad.push(format!("the actor: player {} direct control {direct}", e.is_player()));
                }
                players += 1;
            } else if e.is_player() {
                bad.push(format!("spawn {i} is marked as the player"));
            }
        }
        if players != 1 {
            bad.push(format!("{players} actors"));
        }
        bad
    }

    #[test]
    fn editor_spawn_holds_what_the_games_loader_relies_on() {
        let st = settings();
        let lg = grid(6, 2, [1; 16]);
        let nav = LevelNav::new(&lg);
        let points = vec![
            GraphPoint {
                name: "gp_a".into(),
                position: Vec3::new(0.0, 0.0, 0.0),
                locations: [0; 4],
                connection_level: String::new(),
                connection_point: String::new(),
            },
            GraphPoint {
                name: "gp_b".into(),
                position: Vec3::new(5.0, 0.0, 1.0),
                locations: [0; 4],
                connection_level: String::new(),
                connection_point: String::new(),
            },
        ];
        let gg = build_game_graph(&nav, &points, "l_test", lg.header.guid, [2; 16]).unwrap();
        let element = |section: &str, name: &str, p: Vec3| {
            Element::new(Entity::create_spawn_point(section, &st).unwrap(), name, p, Vec3::ZERO, None)
        };
        let elements = vec![
            element("graph_point", "gp_a", points[0].position),
            element("graph_point", "gp_b", points[1].position),
            element("actor", "actor", Vec3::new(4.0, 0.0, 1.0)),
            element("medkit", "medkit", Vec3::new(1.0, 0.0, 0.0)),
        ];
        let built = build_spawn("l_test", &nav, &gg.graph, elements, &[], [3; 16], false).unwrap();
        let bad = check_spawn_file(&built.bytes, "l_test", &lg.write().unwrap(), &st);
        assert!(bad.is_empty(), "{bad:#?}");
        // the actor stands on b's half of the map, the medkit on a's
        let file = SpawnFile::parse(Arc::from(&built.bytes[..])).unwrap();
        let ents: Vec<Entity> =
            file.vertices.iter().map(|v| Entity::from_packets(&v.spawn, &v.update, &st).unwrap()).collect();
        assert_eq!(ents.iter().map(|e| e.section()).collect::<Vec<_>>(), ["actor", "medkit"]);
        assert_eq!(ents[0].game_vertex_id(), Some(1));
        assert_eq!(ents[1].game_vertex_id(), Some(0));
    }

    #[test]
    fn editor_spawn_on_the_linked_games_graph_holds_the_same() {
        let st = settings();
        let lg = grid(6, 2, [1; 16]);
        let nav = LevelNav::new(&lg);
        let point = |name: &str, x: f32, z: f32| GraphPoint {
            name: name.into(),
            position: Vec3::new(x, 0.0, z),
            locations: [0; 4],
            connection_level: String::new(),
            connection_point: String::new(),
        };
        let own = build_game_graph(
            &nav,
            &[point("gp_a", 0.0, 0.0), point("gp_b", 5.0, 1.0)],
            "l_test",
            lg.header.guid,
            [2; 16],
        )
        .unwrap();
        let base = crate::graph::tests::base_world();
        let joined = crate::graph::join_base(&base, &own.graph).unwrap().unwrap();
        let elements = vec![Element::new(
            Entity::create_spawn_point("actor", &st).unwrap(),
            "actor",
            Vec3::new(4.0, 0.0, 1.0),
            Vec3::ZERO,
            None,
        )];
        let built = build_spawn("l_test", &nav, &joined, elements, &[], [3; 16], false).unwrap();
        let bad = check_spawn_file(&built.bytes, "l_test", &lg.write().unwrap(), &st);
        assert!(bad.is_empty(), "{bad:#?}");
        // the actor on the level's own vertex b, numbered after the game's vertices
        let file = SpawnFile::parse(Arc::from(&built.bytes[..])).unwrap();
        let actor = Entity::from_packets(&file.vertices[0].spawn, &file.vertices[0].update, &st).unwrap();
        assert_eq!(actor.game_vertex_id(), Some(base.vertex_count + 1));
        assert_eq!(file.game_graph.unwrap().guid, base.guid);
    }

    #[test]
    fn an_actor_saved_without_the_player_mark_is_the_player_in_editor_spawn() {
        let st = settings();
        let lg = grid(3, 1, [1; 16]);
        let nav = LevelNav::new(&lg);
        let point = GraphPoint {
            name: "gp".into(),
            position: Vec3::ZERO,
            locations: [0; 4],
            connection_level: String::new(),
            connection_point: String::new(),
        };
        let gg = build_game_graph(&nav, &[point], "l_test", lg.header.guid, [2; 16]).unwrap();
        // what a scene placed before the mark was made keeps: the factory's entity
        let unmarked = Entity::create("actor", &st).unwrap();
        assert!(!unmarked.is_player());
        let elements = vec![Element::new(unmarked, "actor", Vec3::new(1.0, 0.0, 0.0), Vec3::ZERO, None)];
        let built = build_spawn("l_test", &nav, &gg.graph, elements, &[], [3; 16], false).unwrap();
        let bad = check_spawn_file(&built.bytes, "l_test", &lg.write().unwrap(), &st);
        assert!(bad.is_empty(), "{bad:#?}");
    }

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
