//! The AI map as a level graph, the level's game graph with its cross table
//! , and a game graph of several levels.

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::Vec3;
use gz_base::{Finding, Findings};
use gz_fmt_level::{AiMapNode, AiParams, LevelGraph, XRAI_VERSION_COP, export_level_graph};
use gz_fmt_spawn::{Bytes, CrossTable, GameGraph, GraphEdge, GraphLevel, GraphVertex, Guid, LevelPoint};

use crate::nav::{INVALID, LevelNav};
use crate::{BuildError, NEVER, Result, Stop};

/// The game graph's vertex record and edge record sizes (`sizeof(CVertex)`, `sizeof(CEdge)`).
const VERTEX_SIZE: u32 = 42;
const EDGE_SIZE: u32 = 6;
const POINT_SIZE: u32 = 20;
/// `EPS_L`.
const EPS_L: f32 = 0.0001;

/// A vertex of the merged game graph: its level, position, level vertex, types, edges and
/// death points.
type MergedVertex = (u8, Vec3, u32, [u8; 4], Vec<(u16, f32)>, Vec<LevelPoint>);

/// The level graph of the AI Map tool's nodes (CoP layout).
pub fn level_graph(nodes: &[AiMapNode], params: &AiParams, guid: Guid) -> Result<LevelGraph> {
    if nodes.is_empty() {
        return Err(BuildError::new("The AI map is empty."));
    }
    export_level_graph(nodes, params, guid, XRAI_VERSION_COP).map_err(|e| BuildError::new(e.to_string()))
}

/// A graph point of the scene.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphPoint {
    pub name: String,
    pub position: Vec3,
    pub locations: [u8; 4],
    pub connection_level: String,
    pub connection_point: String,
}

/// One vertex of the graph being built, with its edges sorted by the vertex they lead to.
#[derive(Clone, Debug)]
struct Vertex {
    point: Vec3,
    level_vertex: u32,
    types: [u8; 4],
    edges: Vec<(u32, f32)>,
}

impl Vertex {
    fn edge(&self, to: u32) -> Option<f32> {
        self.edges.binary_search_by_key(&to, |e| e.0).ok().map(|i| self.edges[i].1)
    }

    fn add_edge(&mut self, to: u32, w: f32) {
        if let Err(i) = self.edges.binary_search_by_key(&to, |e| e.0) {
            self.edges.insert(i, (to, w));
        }
    }

    fn remove_edge(&mut self, to: u32) {
        if let Ok(i) = self.edges.binary_search_by_key(&to, |e| e.0) {
            self.edges.remove(i);
        }
    }
}

/// What the level's game graph build made.
#[derive(Clone, Debug)]
pub struct LevelGameGraph {
    /// One level, id 0, its cross table attached.
    pub graph: GameGraph,
    /// The graph points that became vertices, in vertex order.
    pub points: Vec<GraphPoint>,
    /// What the build did and what it dropped.
    pub log: Vec<Finding>,
}

/// The graph points on the AI map, each level vertex given to
/// the nearest of them (the cross table), edges between points whose areas touch with their path
/// lengths, then every edge a shorter detour makes redundant taken out.
pub fn build_game_graph(
    nav: &LevelNav,
    points: &[GraphPoint],
    level_name: &str,
    level_guid: Guid,
    game_guid: Guid,
) -> Result<LevelGameGraph> {
    build_game_graph_until(nav, points, level_name, level_guid, game_guid, NEVER)
}

/// [`build_game_graph`] that ends with [`BuildError::stopped`] once `stop` says so: asked per
/// graph point.
pub fn build_game_graph_until(
    nav: &LevelNav,
    points: &[GraphPoint],
    level_name: &str,
    level_guid: Guid,
    game_guid: Guid,
    stop: Stop<'_>,
) -> Result<LevelGameGraph> {
    let mut log = vec![
        Finding::info("Building level game graph"),
        Finding::info(format!("{} nodes loaded", nav.count())),
    ];
    let mut vertices: Vec<Vertex> = Vec::new();
    let mut kept: Vec<GraphPoint> = Vec::new();
    for p in points {
        let near = vertices.iter().any(|v| v.point.distance_squared(p.position) < EPS_L);
        let fmt = |why: &str| {
            format!(
                "Graph point [{}][{:.6}][{:.6}][{:.6}] is removed because {why}.",
                p.name, p.position.x, p.position.y, p.position.z
            )
        };
        if near {
            log.warn(fmt("it is too close to another graph point"));
            continue;
        }
        let node = if nav.valid_position(p.position) { nav.vertex_id(p.position) } else { INVALID };
        if !nav.valid_id(node) {
            log.warn(fmt("it is outside the AI map"));
            continue;
        }
        if vertices.iter().any(|v| v.level_vertex == node) {
            log.warn(fmt("it has the same AI node as another graph point"));
            continue;
        }
        vertices.push(Vertex {
            point: p.position,
            level_vertex: node,
            types: p.locations,
            edges: Vec::new(),
        });
        kept.push(p.clone());
    }
    log.info(format!("{} graph points loaded", vertices.len()));
    if vertices.is_empty() {
        return Err(BuildError::new("The level has no game graph points."));
    }
    if vertices.len() >= usize::from(u16::MAX) {
        return Err(BuildError::new("The level has too many game graph points."));
    }
    log.info("Building cross table");
    let (results, distances) = cross_distances(nav, &vertices, stop).ok_or_else(BuildError::stopped)?;
    let cell = nav.header.cell_size;
    let mut cells: Vec<u8> = Vec::with_capacity(nav.count() as usize * 6);
    for (i, &g) in results.iter().enumerate().take(nav.count() as usize) {
        let d = distances.get(g as usize).and_then(|row| row.get(i)).copied().unwrap_or(u32::MAX);
        cells.extend_from_slice(&(g as u16).to_le_bytes());
        cells.extend_from_slice(&(d as f32 * cell).to_le_bytes());
    }
    drop(distances);

    log.info("Building graph");
    log.info("Generating edges");
    for v in 0..vertices.len() {
        if stop() {
            return Err(BuildError::stopped());
        }
        let neighbours = fill_neighbours(nav, &results, &vertices, v as u32);
        for nb in neighbours {
            let d = path_distance(nav, &vertices, v as u32, nb, &mut log)?;
            vertices[v].add_edge(nb, d);
        }
    }
    let edge_count: usize = vertices.iter().map(|v| v.edges.len()).sum();
    log.info(format!("{edge_count} edges built"));
    log.info("Checking graph connectivity");
    log.info("Optimizing graph");
    log.info(format!("edges before optimization : {edge_count}"));
    optimize(&mut vertices);
    let edge_count: usize = vertices.iter().map(|v| v.edges.len()).sum();
    log.info(format!("edges after optimization : {edge_count}"));

    let graph = assemble(
        &vertices,
        game_guid,
        vec![GraphLevel {
            name: level_name.into(),
            offset: Vec3::ZERO,
            id: 0,
            section: "".into(),
            guid: level_guid,
        }],
        0,
        Vec::new(),
        vec![CrossTable {
            version: XRAI_VERSION_COP,
            level_vertex_count: nav.count(),
            game_vertex_count: vertices.len() as u32,
            level_guid,
            game_guid,
            cells: Bytes::from_vec(cells),
        }],
    )?;
    log.info("Level graph is generated successfully");
    Ok(LevelGameGraph { graph, points: kept, log })
}

/// `fill_marks` + `fill_distances` + `iterate_distances`: per level vertex the game vertex that
/// owns it, and per game vertex its walking distance in cells to every level vertex it owns.
fn cross_distances(nav: &LevelNav, vertices: &[Vertex], stop: Stop<'_>) -> Option<(Vec<u32>, Vec<Vec<u32>>)> {
    let n = nav.count() as usize;
    // what no graph point reaches stays marked: the walks never enter it
    let mut marks = vec![false; n];
    let mut stack: Vec<u32> = Vec::with_capacity(8192);
    for v in vertices {
        stack.push(v.level_vertex);
        while let Some(id) = stack.pop() {
            marks[id as usize] = true;
            for next in nav.links(id) {
                if nav.valid_id(next) && !marks[next as usize] {
                    stack.push(next);
                }
            }
        }
    }
    for m in &mut marks {
        *m = !*m;
    }
    let mut distances: Vec<Vec<u32>> = vec![vec![u32::MAX; n]; vertices.len()];
    let mut results = vec![0u32; n];
    let (mut current, mut next_fringe): (Vec<u32>, Vec<u32>) = (Vec::new(), Vec::new());
    for g in 0..vertices.len() {
        if stop() {
            return None;
        }
        let lv = vertices[g].level_vertex;
        let owner = results[lv as usize] as usize;
        distances[owner][lv as usize] = u32::MAX;
        distances[g].fill(u32::MAX);
        current.clear();
        current.push(lv);
        let mut curr_dist = 0u32;
        while !current.is_empty() {
            for &id in &current {
                results[id as usize] = g as u32;
                distances[g][id as usize] = curr_dist;
                for nx in nav.links(id) {
                    if !nav.valid_id(nx) || marks[nx as usize] {
                        continue;
                    }
                    if distances[g][nx as usize] <= curr_dist {
                        continue;
                    }
                    let owner = results[nx as usize] as usize;
                    if distances[owner][nx as usize] <= curr_dist + 1 {
                        continue;
                    }
                    next_fringe.push(nx);
                    marks[nx as usize] = true;
                }
            }
            for &id in &current {
                marks[id as usize] = false;
            }
            std::mem::swap(&mut current, &mut next_fringe);
            next_fringe.clear();
            curr_dist += 1;
        }
    }
    Some((results, distances))
}

/// `fill_neighbours`: the game vertices whose areas border this vertex's, in the order a walk
/// over its own area meets them.
fn fill_neighbours(nav: &LevelNav, owner: &[u32], vertices: &[Vertex], g: u32) -> Vec<u32> {
    let mut marks = vec![false; nav.count() as usize];
    let mut out: Vec<u32> = Vec::new();
    let mut stack = vec![vertices[g as usize].level_vertex];
    while let Some(id) = stack.pop() {
        marks[id as usize] = true;
        for next in nav.links(id) {
            if !nav.valid_id(next) || marks[next as usize] {
                continue;
            }
            let ng = owner[next as usize];
            if ng != g {
                if !out.contains(&ng) {
                    out.push(ng);
                }
                continue;
            }
            stack.push(next);
        }
    }
    out
}

/// `path_distance`: straight when the straight walk stays on the map, the path length otherwise.
fn path_distance(nav: &LevelNav, vertices: &[Vertex], a: u32, b: u32, log: &mut Vec<Finding>) -> Result<f32> {
    let (v0, v1) = (&vertices[a as usize], &vertices[b as usize]);
    let pure = glam::Vec2::new(v0.point.x - v1.point.x, v0.point.z - v1.point.z).length();
    if nav.valid_id(nav.check_position_in_direction(v0.level_vertex, v0.point, v1.point)) {
        return Ok(pure);
    }
    match nav.path_distance(v0.level_vertex, v0.point, v1.level_vertex, v1.point) {
        Some(d) => Ok(d),
        None => {
            log.error(format!("No path from graph point [{a}] to [{b}]."));
            Err(BuildError::new(format!(
                "Cannot build path from [{:.6}][{:.6}][{:.6}] to [{:.6}][{:.6}][{:.6}], check AI map",
                v0.point.x, v0.point.y, v0.point.z, v1.point.x, v1.point.y, v1.point.z
            )))
        }
    }
}

/// `optimize_graph`: an edge whose two ends share a third neighbour goes, the longest first.
fn optimize(vertices: &mut [Vertex]) {
    let mut triples: Vec<(f32, u32, u32)> = Vec::new();
    for (i, v) in vertices.iter().enumerate() {
        let i = i as u32;
        for &(to, w) in &v.edges {
            if to < i && vertices[to as usize].edge(i).is_some() {
                continue;
            }
            let back = vertices[to as usize].edge(i);
            triples.push((w.min(back.unwrap_or(w)), i, to));
        }
    }
    // std::sort with a greater-than predicate leaves equal weights in an unspecified
    // order; here they keep the order they were made in
    triples.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, v0, v1) in triples {
        let others: Vec<u32> = vertices[v0 as usize].edges.iter().map(|e| e.0).filter(|&t| t != v1).collect();
        for t in others {
            let common = vertices[v1 as usize].edge(t).is_some() || vertices[t as usize].edge(v1).is_some();
            if common {
                vertices[v0 as usize].remove_edge(v1);
                vertices[v1 as usize].remove_edge(v0);
                break;
            }
        }
    }
}

/// The file form of a graph: edge offsets after the vertex records, death point offsets after
/// the edges.
fn assemble(
    vertices: &[Vertex],
    guid: Guid,
    levels: Vec<GraphLevel>,
    level_id: u8,
    death: Vec<(u8, Vec<LevelPoint>)>,
    cross: Vec<CrossTable>,
) -> Result<GameGraph> {
    let count = u32::try_from(vertices.len()).map_err(|_| BuildError::new("too many graph vertices"))?;
    let edges: u32 = vertices.iter().map(|v| v.edges.len() as u32).sum();
    let mut edge_offset = count * VERTEX_SIZE;
    let mut out_v = Vec::with_capacity(vertices.len());
    let mut out_e = Vec::with_capacity(edges as usize);
    for v in vertices {
        if v.edges.len() > 255 {
            return Err(BuildError::new("a graph vertex has more than 255 edges"));
        }
        out_v.push(GraphVertex {
            local_point: v.point,
            global_point: v.point,
            level_id,
            level_vertex: v.level_vertex,
            vertex_types: v.types,
            edge_offset,
            point_offset: 0,
            edge_count: v.edges.len() as u8,
            death_point_count: 0,
        });
        edge_offset += v.edges.len() as u32 * EDGE_SIZE;
        out_e.extend(v.edges.iter().map(|&(to, w)| GraphEdge { vertex_id: to as u16, distance: w }));
    }
    let death_points: Vec<LevelPoint> = death.into_iter().flat_map(|(_, p)| p).collect();
    Ok(GameGraph {
        version: XRAI_VERSION_COP as u8,
        vertex_count: count as u16,
        edge_count: edges,
        death_point_count: death_points.len() as u32,
        guid,
        levels,
        vertices: out_v,
        edges: out_e,
        death_points,
        cross_tables: cross,
        tail: Bytes::from_vec(Vec::new()),
    })
}

/// The first level id a level added to the linked game's graph takes (the
/// game's own levels live well below it).
const JOINED_LEVEL_ID: u8 = 128;
/// Where an added level lies in global space (away from the base map).
const JOINED_OFFSET: Vec3 = Vec3::new(10000.0, 0.0, 10000.0);

/// Play's game graph: the level's own graph appended to the linked game's, the way the game
/// adds a module's level. The game's vertices, their ids and the
/// graph's guid stay as they are - the game's scripts name its vertices by number - and the level
/// follows them under a level id of its own, its vertices and cross table renumbered after them.
/// `None` when the linked game already has a level of that name: the game's composer adds no level
/// of the base game either.
pub fn join_base(base: &GameGraph, level: &GameGraph) -> Result<Option<GameGraph>> {
    let (Some(own), Some(own_cross)) = (level.levels.first(), level.cross_tables.first()) else {
        return Err(BuildError::new("The level's game graph has no level."));
    };
    if base.levels.iter().any(|l| l.name.eq_ignore_ascii_case(&own.name)) {
        return Ok(None);
    }
    if base.cross_tables.len() != base.levels.len() {
        return Err(BuildError::new("The linked game's graph has no cross table for every level."));
    }
    let id = base.levels.iter().map(|l| l.id.saturating_add(1)).fold(JOINED_LEVEL_ID, u8::max);
    if id == u8::MAX {
        return Err(BuildError::new("The linked game's graph has no free level id."));
    }
    let first = base.vertices.len();
    let total = first + level.vertices.len();
    let count = u16::try_from(total).ok().filter(|&n| n < u16::MAX).ok_or_else(|| {
        BuildError::new("The linked game's graph and the level hold too many game vertices together.")
    })?;
    // per vertex: the record, its edges, its death points
    let mut joined: Vec<(GraphVertex, Vec<GraphEdge>, Vec<LevelPoint>)> = Vec::with_capacity(total);
    for (i, v) in base.vertices.iter().enumerate() {
        let i = i as u16;
        joined.push((*v, base.edges_of(i).to_vec(), base.points_of(i).to_vec()));
    }
    for (i, v) in level.vertices.iter().enumerate() {
        let edges = level
            .edges_of(i as u16)
            .iter()
            .map(|e| GraphEdge { vertex_id: e.vertex_id.saturating_add(first as u16), distance: e.distance })
            .collect();
        let vertex = GraphVertex { global_point: v.local_point + JOINED_OFFSET, level_id: id, ..*v };
        joined.push((vertex, edges, Vec::new()));
    }
    let edge_count: u32 = joined.iter().map(|j| j.1.len() as u32).sum();
    let point_count: u32 = joined.iter().map(|j| j.2.len() as u32).sum();
    let mut edge_offset = total as u32 * VERTEX_SIZE;
    let mut point_offset = edge_offset + edge_count * EDGE_SIZE;
    let (mut vertices, mut edges, mut death_points) =
        (Vec::with_capacity(total), Vec::with_capacity(edge_count as usize), Vec::new());
    for (v, e, p) in joined {
        vertices.push(GraphVertex {
            edge_offset,
            point_offset,
            edge_count: e.len() as u8,
            death_point_count: p.len() as u8,
            ..v
        });
        edge_offset += e.len() as u32 * EDGE_SIZE;
        point_offset += p.len() as u32 * POINT_SIZE;
        edges.extend(e);
        death_points.extend(p);
    }
    let mut cells = Vec::with_capacity(own_cross.cells.len());
    for n in 0..own_cross.level_vertex_count {
        let (gv, d) = own_cross.cell(n).unwrap_or((0, 0.0));
        cells.extend_from_slice(&gv.saturating_add(first as u16).to_le_bytes());
        cells.extend_from_slice(&d.to_le_bytes());
    }
    // the game walks the cross tables in the order of the level ids
    let mut tables: Vec<(GraphLevel, CrossTable)> =
        base.levels.iter().cloned().zip(base.cross_tables.iter().cloned()).collect();
    tables.sort_by_key(|(l, _)| l.id);
    tables.push((
        GraphLevel { name: own.name.clone(), offset: JOINED_OFFSET, id, section: "".into(), guid: own.guid },
        CrossTable {
            game_vertex_count: u32::from(count),
            game_guid: base.guid,
            cells: Bytes::from_vec(cells),
            ..own_cross.clone()
        },
    ));
    let (levels, cross_tables) = tables.into_iter().unzip();
    Ok(Some(GameGraph {
        version: base.version,
        vertex_count: count,
        edge_count,
        death_point_count: point_count,
        guid: base.guid,
        levels,
        vertices,
        edges,
        death_points,
        cross_tables,
        tail: Bytes::from_vec(Vec::new()),
    }))
}

/// One level of a merge: its own game graph, the graph points it was made of and the
/// caption other levels' connections name it by.
#[derive(Clone, Debug)]
pub struct LevelPart {
    pub id: u8,
    pub name: String,
    /// `caption` of the level's section in `game.ltx`: what a connection names.
    pub caption: String,
    pub section: String,
    pub offset: Vec3,
    pub graph: LevelGameGraph,
    pub nav: Arc<LevelNav>,
}

/// The levels' graphs one after another (by level id), every vertex in world
/// space, the death points of each vertex, the connection points joined by edges, the cross
/// tables renumbered.
// the X-Ray SDK picks a vertex's death points with `random_shuffle`; here they are the
// first ones of its area, so a merge is reproducible.
pub fn merge_levels(parts: &[LevelPart], guid: Guid) -> Result<(GameGraph, Vec<Finding>)> {
    let mut log = Vec::new();
    let mut parts: Vec<&LevelPart> = parts.iter().collect();
    parts.sort_by_key(|p| p.id);
    if parts.is_empty() {
        return Err(BuildError::new("There are no levels to merge."));
    }
    let mut offsets = BTreeMap::new();
    let mut offset = 0u32;
    for p in &parts {
        if offsets.insert(p.id, offset).is_some() {
            return Err(BuildError::new("Level ids _MUST_ be different!"));
        }
        offset += p.graph.graph.vertices.len() as u32;
    }
    let total = offset;
    // per level: the vertices with their edges renumbered, the death points, the connections
    let mut merged: Vec<MergedVertex> = Vec::new();
    for p in &parts {
        let off = offsets[&p.id];
        let g = &p.graph.graph;
        let cross = g.cross_tables.first();
        for (i, v) in g.vertices.iter().enumerate() {
            let edges: Vec<(u16, f32)> = g
                .edges_of(i as u16)
                .iter()
                .map(|e| ((u32::from(e.vertex_id) + off) as u16, e.distance))
                .collect();
            let mut nodes: Vec<u32> = Vec::new();
            if let Some(c) = cross {
                for n in 0..c.level_vertex_count {
                    if c.cell(n).is_some_and(|(gv, _)| u32::from(gv) == i as u32) {
                        nodes.push(n);
                    }
                }
            }
            let m = if nodes.len() > 10 {
                ((0.1 * nodes.len() as f32).floor() as usize).min(255)
            } else {
                nodes.len()
            };
            let points = nodes
                .iter()
                .take(m)
                .map(|&n| LevelPoint {
                    point: p.nav.vertex_position(n),
                    level_vertex: n,
                    distance: cross.and_then(|c| c.cell(n)).map_or(0.0, |c| c.1),
                })
                .collect();
            merged.push((p.id, v.global_point + p.offset, v.level_vertex, v.vertex_types, edges, points));
        }
    }
    // interconnection points: each level's points by name, in name order
    for p in &parts {
        let off = offsets[&p.id];
        let mut by_name: Vec<(&GraphPoint, u32)> =
            p.graph.points.iter().enumerate().map(|(i, gp)| (gp, i as u32)).collect();
        by_name.sort_by(|a, b| a.0.name.cmp(&b.0.name));
        for (gp, local) in by_name {
            if gp.connection_point.is_empty() {
                continue;
            }
            let Some(target) = parts.iter().find(|t| t.caption == gp.connection_level) else {
                log.warn(format!(
                    "Level {} is not part of the merge: its connection point is not generated.",
                    gp.connection_level
                ));
                continue;
            };
            let Some(t_local) = target.graph.points.iter().position(|t| t.name == gp.connection_point) else {
                return Err(BuildError::new(format!(
                    "Level {} with id {} has an INVALID connection point {}, which references to graph point {} on the level {} with id {}",
                    p.name, p.id, gp.name, gp.connection_point, target.name, target.id
                )));
            };
            let to = offsets[&target.id] + t_local as u32;
            let from = (off + local) as usize;
            let distance = merged[from].1.distance(merged[to as usize].1);
            log.info(format!(
                "Level {} with id {} has VALID connection point {}, which references to graph point {} on the level {} with id {}",
                p.name, p.id, gp.name, gp.connection_point, target.name, target.id
            ));
            merged[from].4.push((to as u16, distance));
        }
    }
    let edge_count: u32 = merged.iter().map(|m| m.4.len() as u32).sum();
    let mut edge_offset = total * VERTEX_SIZE;
    let mut point_offset = edge_offset + edge_count * EDGE_SIZE;
    let mut vertices = Vec::with_capacity(merged.len());
    let mut edges = Vec::with_capacity(edge_count as usize);
    let mut death_points = Vec::new();
    for p in &parts {
        let off = offsets[&p.id] as usize;
        for (i, v) in p.graph.graph.vertices.iter().enumerate() {
            let m = &merged[off + i];
            if m.4.len() > 255 || m.5.len() > 255 {
                return Err(BuildError::new("a graph vertex has more than 255 edges or death points"));
            }
            vertices.push(GraphVertex {
                local_point: v.local_point,
                global_point: m.1,
                level_id: m.0,
                level_vertex: m.2,
                vertex_types: m.3,
                edge_offset,
                point_offset,
                edge_count: m.4.len() as u8,
                death_point_count: m.5.len() as u8,
            });
            edge_offset += m.4.len() as u32 * EDGE_SIZE;
            point_offset += m.5.len() as u32 * POINT_SIZE;
        }
    }
    for m in &merged {
        edges.extend(m.4.iter().map(|&(to, d)| GraphEdge { vertex_id: to, distance: d }));
    }
    for m in &merged {
        death_points.extend(m.5.iter().cloned());
    }
    let mut cross_tables = Vec::new();
    for p in &parts {
        let off = offsets[&p.id];
        if let Some(c) = p.graph.graph.cross_tables.first() {
            let mut cells = Vec::with_capacity(c.cells.len());
            for n in 0..c.level_vertex_count {
                let (gv, d) = c.cell(n).unwrap_or((0, 0.0));
                cells.extend_from_slice(&((u32::from(gv) + off) as u16).to_le_bytes());
                cells.extend_from_slice(&d.to_le_bytes());
            }
            cross_tables.push(CrossTable { game_guid: guid, cells: Bytes::from_vec(cells), ..c.clone() });
        }
    }
    let levels = parts
        .iter()
        .map(|p| GraphLevel {
            name: p.name.as_str().into(),
            offset: p.offset,
            id: p.id,
            section: p.section.as_str().into(),
            guid: p.graph.graph.levels.first().map_or([0; 16], |l| l.guid),
        })
        .collect();
    let graph = GameGraph {
        version: XRAI_VERSION_COP as u8,
        vertex_count: u16::try_from(total).map_err(|_| BuildError::new("too many graph vertices"))?,
        edge_count,
        death_point_count: death_points.len() as u32,
        guid,
        levels,
        vertices,
        edges,
        death_points,
        cross_tables,
        tail: Bytes::from_vec(Vec::new()),
    };
    Ok((graph, log))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use gz_base::Severity;
    use gz_fmt_level::AiMapNode;

    /// A `w` x `h` grid of flat nodes, one metre apart, linked to their neighbours.
    pub(crate) fn grid(w: u32, h: u32, guid: Guid) -> LevelGraph {
        let mut nodes = Vec::new();
        for x in 0..w {
            for z in 0..h {
                let mut n = AiMapNode::new(Vec3::new(x as f32, 0.0, z as f32), Vec3::Y);
                let id = |x: u32, z: u32| x * h + z;
                // left, forward, right, back
                n.links = [
                    (x > 0).then(|| id(x - 1, z)),
                    (z + 1 < h).then(|| id(x, z + 1)),
                    (x + 1 < w).then(|| id(x + 1, z)),
                    (z > 0).then(|| id(x, z - 1)),
                ];
                nodes.push(n);
            }
        }
        let params = AiParams { patch_size: 1.0, test_height: 1.0, can_up: 1.5, can_down: 4.0 };
        level_graph(&nodes, &params, guid).unwrap()
    }

    fn point(name: &str, x: f32, z: f32, level: &str, to: &str) -> GraphPoint {
        GraphPoint {
            name: name.into(),
            position: Vec3::new(x, 0.0, z),
            locations: [1, 2, 3, 4],
            connection_level: level.into(),
            connection_point: to.into(),
        }
    }

    #[test]
    fn a_level_graph_has_its_cross_table() {
        let lg = grid(6, 2, [1; 16]);
        let nav = LevelNav::new(&lg);
        let points = vec![
            point("a", 0.0, 0.0, "", ""),
            point("b", 5.0, 0.0, "", ""),
            point("too_close", 0.0, 0.0, "", ""),
        ];
        let g = build_game_graph(&nav, &points, "l01", [1; 16], [2; 16]).unwrap();
        assert_eq!(g.graph.vertices.len(), 2);
        assert_eq!(g.points.len(), 2);
        assert!(g.log.iter().any(|l| l.severity == Severity::Warning
            && l.text.contains("too_close")
            && l.text.contains("too close")));
        // a and b share a border: one edge each way, the straight distance
        assert_eq!(g.graph.edges.len(), 2);
        assert!((g.graph.edges[0].distance - 5.0).abs() < 1e-4);
        let ct = &g.graph.cross_tables[0];
        assert_eq!(ct.level_vertex_count, 12);
        // the left half belongs to a, the right half to b
        let a_cells = (0..12).filter(|&n| ct.cell(n).is_some_and(|c| c.0 == 0)).count();
        assert_eq!(a_cells, 6);
        assert_eq!(g.graph.vertices[0].edge_offset, 2 * VERTEX_SIZE);
        assert_eq!(g.graph.vertices[1].edge_offset, 2 * VERTEX_SIZE + EDGE_SIZE);
    }

    /// A game graph of two levels, as a linked game's `all.spawn` holds one.
    pub(crate) fn base_world() -> GameGraph {
        let (la, lb) = (grid(4, 1, [1; 16]), grid(3, 1, [3; 16]));
        let (na, nb) = (Arc::new(LevelNav::new(&la)), Arc::new(LevelNav::new(&lb)));
        let pa = vec![point("a_start", 0.0, 0.0, "", ""), point("a_exit", 3.0, 0.0, "level_b", "b_entry")];
        let pb = vec![point("b_entry", 0.0, 0.0, "level_a", "a_exit"), point("b_far", 2.0, 0.0, "", "")];
        let ga = build_game_graph(&na, &pa, "l_a", [1; 16], [2; 16]).unwrap();
        let gb = build_game_graph(&nb, &pb, "l_b", [3; 16], [4; 16]).unwrap();
        let part = |id: u8, name: &str, caption: &str, graph: LevelGameGraph, nav: Arc<LevelNav>| LevelPart {
            id,
            name: name.into(),
            caption: caption.into(),
            section: format!("level{id}"),
            offset: Vec3::new(f32::from(id) * 100.0, 0.0, 0.0),
            graph,
            nav,
        };
        let parts = vec![part(1, "l_a", "level_a", ga, na), part(2, "l_b", "level_b", gb, nb)];
        merge_levels(&parts, [9; 16]).unwrap().0
    }

    #[test]
    fn a_level_is_appended_to_the_linked_games_graph_with_its_ids_unchanged() {
        let base = base_world();
        let lg = grid(6, 2, [5; 16]);
        let nav = LevelNav::new(&lg);
        let own = build_game_graph(
            &nav,
            &[point("x", 0.0, 0.0, "", ""), point("y", 5.0, 1.0, "", "")],
            "l_new",
            [5; 16],
            [6; 16],
        )
        .unwrap()
        .graph;
        let g = join_base(&base, &own).unwrap().unwrap();
        // the game's vertices, edges and death points where they were, the graph's guid kept
        let n = base.vertices.len();
        assert_eq!(g.guid, base.guid);
        assert_eq!(usize::from(g.vertex_count), n + own.vertices.len());
        for i in 0..n as u16 {
            let (a, b) = (&base.vertices[usize::from(i)], &g.vertices[usize::from(i)]);
            assert_eq!(
                (a.local_point, a.global_point, a.level_id, a.level_vertex),
                (b.local_point, b.global_point, b.level_id, b.level_vertex)
            );
            assert_eq!(base.edges_of(i), g.edges_of(i));
            assert_eq!(base.points_of(i), g.points_of(i));
        }
        assert!(!base.death_points.is_empty() && g.death_points.len() == base.death_points.len());
        // the level under an id of its own, after the game's, with its cross table renumbered
        let l = g.levels.last().unwrap();
        assert_eq!((l.name.as_str(), l.id, l.guid), ("l_new", JOINED_LEVEL_ID, [5; 16]));
        assert_eq!(g.levels.iter().map(|l| l.id).collect::<Vec<_>>(), [1, 2, JOINED_LEVEL_ID]);
        let ct = g.cross_table(JOINED_LEVEL_ID).unwrap();
        assert_eq!((ct.game_guid, ct.level_guid, ct.level_vertex_count), (base.guid, [5; 16], 12));
        assert_eq!(ct.game_vertex_count, u32::from(g.vertex_count));
        for c in 0..12 {
            let (gv, _) = ct.cell(c).unwrap();
            assert_eq!(usize::from(gv), n + usize::from(own.cross_tables[0].cell(c).unwrap().0));
        }
        let mine = &g.vertices[n];
        assert_eq!(
            (mine.level_id, mine.global_point),
            (JOINED_LEVEL_ID, own.vertices[0].local_point + JOINED_OFFSET)
        );
        assert_eq!(g.edges_of(n as u16)[0].vertex_id as usize, n + 1);
        // the file reads back as it was made, and a level the game already has is not added again
        let back = GameGraph::parse_file(&Arc::from(g.to_bytes())).unwrap();
        assert_eq!(back, g);
        let again = build_game_graph(&nav, &[point("x", 0.0, 0.0, "", "")], "L_A", [5; 16], [6; 16]).unwrap();
        assert!(join_base(&base, &again.graph).unwrap().is_none());
    }

    #[test]
    fn two_levels_are_joined_at_their_connection_points() {
        let (la, lb) = (grid(4, 1, [1; 16]), grid(3, 1, [3; 16]));
        let (na, nb) = (Arc::new(LevelNav::new(&la)), Arc::new(LevelNav::new(&lb)));
        let pa = vec![point("a_start", 0.0, 0.0, "", ""), point("a_exit", 3.0, 0.0, "level_b", "b_entry")];
        let pb = vec![point("b_entry", 0.0, 0.0, "level_a", "a_exit"), point("b_far", 2.0, 0.0, "", "")];
        let ga = build_game_graph(&na, &pa, "l_a", [1; 16], [2; 16]).unwrap();
        let gb = build_game_graph(&nb, &pb, "l_b", [3; 16], [4; 16]).unwrap();
        let parts = vec![
            LevelPart {
                id: 2,
                name: "l_b".into(),
                caption: "level_b".into(),
                section: "level2".into(),
                offset: Vec3::new(100.0, 0.0, 0.0),
                graph: gb,
                nav: nb,
            },
            LevelPart {
                id: 1,
                name: "l_a".into(),
                caption: "level_a".into(),
                section: "level1".into(),
                offset: Vec3::ZERO,
                graph: ga,
                nav: na,
            },
        ];
        let (g, log) = merge_levels(&parts, [9; 16]).unwrap();
        assert_eq!(g.vertex_count, 4);
        assert_eq!(g.levels.iter().map(|l| l.id).collect::<Vec<_>>(), [1, 2]);
        // l_a first (id 1): its exit (vertex 1) leads to l_b's entry (vertex 2) and back
        assert!(g.edges_of(1).iter().any(|e| e.vertex_id == 2));
        assert!(g.edges_of(2).iter().any(|e| e.vertex_id == 1));
        let exit_to_entry = g.edges_of(1).iter().find(|e| e.vertex_id == 2).unwrap().distance;
        assert!((exit_to_entry - 97.0).abs() < 1e-3, "{exit_to_entry}");
        assert_eq!(g.vertices[2].global_point, Vec3::new(100.0, 0.0, 0.0));
        assert_eq!(g.vertices[2].level_id, 2);
        assert_eq!(g.edge_count as usize, g.edges.len());
        assert_eq!(g.cross_tables.len(), 2);
        assert_eq!(g.cross_tables[1].cell(0).unwrap().0, 2);
        assert_eq!(g.cross_tables[1].game_guid, [9; 16]);
        assert_eq!(log.iter().filter(|l| l.text.contains("VALID connection point")).count(), 2);
        // the graph writes and reads back
        let bytes: Arc<[u8]> = g.to_bytes().into();
        let back = GameGraph::parse_file(&bytes).unwrap();
        assert_eq!(back.vertices, g.vertices);
    }
}
