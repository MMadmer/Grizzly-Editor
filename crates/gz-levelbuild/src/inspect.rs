//! `level.inspect`: what a compiled level folder holds, as text - the project's level, a level by
//! name, or any folder, with or without a project. Everything is read off the UI thread, nothing
//! is written, and a long list is paged.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_editor::{CmdError, Editor, JsonSchema, Registry, Reply, Spec, Value, json};
use gz_fmt_level::{Aabb, CForm, CompiledLevel, LevelGraph, Visual, VisualKind};
use gz_scene::need_scene;
use serde::Deserialize;

use crate::service::levels_root;

/// The most rows a list answers at once.
const MAX_LIMIT: usize = 500;
const DEFAULT_LIMIT: usize = 50;
/// How deep and how many entries the file list goes.
const WALK_DEPTH: usize = 4;
const WALK_CAP: usize = 5000;

/// Files that say a folder is a level's: made by a build, or read by the game.
const LEVEL_MARKERS: [&str; 10] = [
    "level",
    "level.cform",
    "level.ai",
    "level.graph",
    "level.geom",
    "level.spawn",
    "level.game",
    "build.prj",
    "build.state",
    "build.report",
];

// Until `cmd::register` calls it, only the tests do.
pub(crate) fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::read(
            "level.inspect",
            "Reads what a level build made, by part (summary, files, visuals, sectors, portals, lights, lightmaps, collision, ai_map, graph, pieces): the open project's level, a level of it by name or any folder.",
        )
        .example("part=visuals level=l01_escape shader=wall limit=20")
        .example("part=pieces level=l01_escape"),
        |ed, p: InspectParams, _o| async move { inspect(ed, p).await },
    );
}

/// Which part of the level to read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Part {
    /// The counts, the box, the files and what build.report says.
    #[default]
    Summary,
    /// Every file of the folder with its size.
    Files,
    /// The visuals: bounds, shader, textures, vertices, faces, lightmap page, sector.
    Visuals,
    Sectors,
    Portals,
    /// The level's dynamic lights and glows.
    Lights,
    /// The lightmap pages and the visuals that use each.
    Lightmaps,
    /// The collision form: triangles by material and sector.
    Collision,
    /// The AI map's header and node count.
    AiMap,
    /// The level's game graph.
    Graph,
    /// The module's pieces of the level (`levels\<level>\pieces`): each one's box, whether it
    /// streams, its visuals, roots, vertices, faces and lightmap pages, and what the game would
    /// refuse it for.
    Pieces,
}

impl Part {
    fn name(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Files => "files",
            Self::Visuals => "visuals",
            Self::Sectors => "sectors",
            Self::Portals => "portals",
            Self::Lights => "lights",
            Self::Lightmaps => "lightmaps",
            Self::Collision => "collision",
            Self::AiMap => "ai_map",
            Self::Graph => "graph",
            Self::Pieces => "pieces",
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InspectParams {
    /// A level folder: absolute, or under the open project. Works without a project when absolute.
    #[serde(default)]
    folder: Option<String>,
    /// A level of the open project (`gamedata\levels\<level>`). With neither folder nor level: the
    /// open scene's level.
    #[serde(default)]
    level: Option<String>,
    /// What to read; left out: summary.
    #[serde(default)]
    part: Part,
    /// visuals: only those whose box meets this one, six numbers: min x, y, z, then max x, y, z.
    #[serde(default, rename = "box")]
    bounds: Option<Vec<f32>>,
    /// visuals: only those under this sector.
    #[serde(default)]
    sector: Option<u32>,
    /// visuals: only those whose shader table entry holds this text (any case).
    #[serde(default)]
    shader: Option<String>,
    /// Skip this many rows first.
    #[serde(default)]
    offset: Option<usize>,
    /// At most this many rows (50, at most 500); the answer says `total`.
    #[serde(default)]
    limit: Option<usize>,
}

struct Query {
    part: Part,
    bounds: Option<Aabb>,
    sector: Option<u32>,
    shader: Option<String>,
    offset: usize,
    limit: usize,
}

async fn inspect(ed: Arc<Editor>, p: InspectParams) -> Reply {
    let query = Query {
        part: p.part,
        bounds: match &p.bounds {
            None => None,
            Some(v) if v.len() == 6 => {
                Some(Aabb::new(glam::Vec3::new(v[0], v[1], v[2]), glam::Vec3::new(v[3], v[4], v[5])))
            }
            Some(v) => {
                return Err(CmdError::invalid(format!(
                    "box: six numbers, min x, y, z then max x, y, z; got {}.",
                    v.len()
                )));
            }
        },
        sector: p.sector,
        shader: p.shader.as_ref().map(|s| s.to_lowercase()),
        offset: p.offset.unwrap_or(0),
        limit: p.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT),
    };
    let (dir, pieces) = resolve(&ed, &p).await?;
    ed.exec.io(move || read_part(&dir, &pieces, &query)).await.map_err(|e| CmdError::new("refused", e))
}

/// The folder the request names, and where the level's pieces are: the module's
/// `levels\<level>\pieces` for a level of the project, a folder's own `pieces` otherwise.
async fn resolve(ed: &Arc<Editor>, p: &InspectParams) -> Result<(PathBuf, PathBuf), CmdError> {
    let of_level = |ed: &Arc<Editor>, level: &str| -> Result<(PathBuf, PathBuf), CmdError> {
        let session = ed.project()?;
        let dir = crate::folder::level_folder(&levels_root(&session), level).map_err(CmdError::invalid)?;
        Ok((dir, crate::pipeline::pieces_dir(&session, level.trim())))
    };
    match (&p.folder, &p.level) {
        (Some(_), Some(_)) => Err(CmdError::invalid("folder or level, not both.")),
        (Some(folder), None) => {
            let raw = PathBuf::from(folder.replace('/', "\\"));
            let dir = if raw.is_absolute() {
                raw
            } else {
                ed.project_root().map(|root| root.join(raw)).ok_or_else(|| {
                    CmdError::invalid("folder: an absolute path, or one inside the open project")
                })?
            };
            let pieces = dir.join("pieces");
            Ok((dir, pieces))
        }
        (None, Some(level)) => of_level(ed, level),
        (None, None) => {
            let svc = need_scene(ed).await?;
            let level = svc.store.snapshot().meta.level.level_path.to_string();
            if level.trim().is_empty() {
                return Err(CmdError::invalid(
                    "The scene has no level path: give a folder or a level, or set it in Level Settings.",
                ));
            }
            of_level(ed, &level)
        }
    }
}

// ------------------------------------------------------------------------------ small helpers

/// A length in metres, three decimals.
fn r3(v: f32) -> f64 {
    (f64::from(v) * 1000.0).round() / 1000.0
}

fn vec3(v: glam::Vec3) -> Value {
    json!([r3(v.x), r3(v.y), r3(v.z)])
}

fn aabb(b: &Aabb) -> Value {
    json!([r3(b.min.x), r3(b.min.y), r3(b.min.z), r3(b.max.x), r3(b.max.y), r3(b.max.z)])
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn show(dir: &Path) -> String {
    dir.display().to_string().replace('\\', "/")
}

/// The bytes of a file; `None` when it is not there.
fn read_file(dir: &Path, name: &str) -> Result<Option<Vec<u8>>, String> {
    match fs::read(dir.join(name)) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!(
            "{name} could not be read: {}.",
            e.raw_os_error().map_or_else(|| e.to_string(), |c| gz_base::os_reason(c as u32))
        )),
    }
}

fn parse<T, E: std::fmt::Display>(name: &str, r: Result<T, E>) -> Result<T, String> {
    r.map_err(|e| format!("{name} could not be read: {e}"))
}

fn missing(part: Part, file: &str) -> Value {
    json!({ "part": part.name(), "missing": file })
}

/// A page of rows with the total.
fn page<T>(rows: impl IntoIterator<Item = T>, q: &Query, key: &str, f: impl Fn(T) -> Value) -> Value {
    let all: Vec<T> = rows.into_iter().collect();
    let total = all.len();
    let rows: Vec<Value> = all.into_iter().skip(q.offset).take(q.limit).map(f).collect();
    json!({ key: rows, "total": total, "offset": q.offset, "limit": q.limit })
}

/// Every file under the folder, relative, with its size; `/` in the names.
pub(crate) fn list_files(dir: &Path) -> Vec<(String, u64)> {
    fn walk(base: &Path, at: &Path, depth: usize, out: &mut Vec<(String, u64)>) {
        let Ok(rd) = fs::read_dir(at) else { return };
        for entry in rd.flatten() {
            if out.len() >= WALK_CAP {
                return;
            }
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if depth < WALK_DEPTH {
                    walk(base, &path, depth + 1, out);
                }
            } else if let Ok(rel) = path.strip_prefix(base) {
                out.push((rel.to_string_lossy().replace('\\', "/"), meta.len()));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, 1, &mut out);
    out.sort();
    out
}

/// Width, height, mip levels, bytes.
type DdsInfo = (u32, u32, u32, u64);

/// A DDS file's width, height, mip levels and size, from its header.
fn dds_info(path: &Path) -> Option<DdsInfo> {
    let mut f = fs::File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    let mut head = [0u8; 32];
    f.read_exact(&mut head).ok()?;
    if &head[..4] != b"DDS " {
        return None;
    }
    let at = |o: usize| u32::from_le_bytes([head[o], head[o + 1], head[o + 2], head[o + 3]]);
    Some((at(16), at(12), at(28), size))
}

/// The lightmap pages the folder holds: page number -> (file name, width, height, mips, bytes).
fn lightmap_files(dir: &Path) -> BTreeMap<u32, Vec<(String, Option<DdsInfo>)>> {
    let mut pages: BTreeMap<u32, Vec<(String, Option<DdsInfo>)>> = BTreeMap::new();
    let Ok(rd) = fs::read_dir(dir) else { return pages };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(rest) = name.strip_prefix("lmap#") else { continue };
        let Some(num) = rest.strip_suffix("_1.dds").or_else(|| rest.strip_suffix("_2.dds")) else { continue };
        let Ok(n) = num.parse::<u32>() else { continue };
        let info = dds_info(&entry.path());
        pages.entry(n).or_default().push((name, info));
    }
    for files in pages.values_mut() {
        files.sort();
    }
    pages
}

// ------------------------------------------------------------------------------------ the counts

/// What a build made, counted from the files. Shared by the summary and `build.report`.
#[derive(Debug, Default)]
pub(crate) struct Counts {
    pub visuals: Option<usize>,
    pub kinds: BTreeMap<String, usize>,
    pub faces: usize,
    pub vertices: usize,
    pub shaders: usize,
    pub sectors: usize,
    pub portals: usize,
    pub lights: usize,
    pub glows: usize,
    pub quality: Option<u16>,
    pub aabb: Option<Aabb>,
    pub collision: Option<(usize, usize, Aabb)>,
    pub lightmap_pages: usize,
    pub ai_nodes: Option<usize>,
    pub graph: Option<(usize, usize)>,
    /// What could not be read.
    pub problems: Vec<String>,
}

impl Counts {
    /// Reads every file of the folder that holds counts. Blocking.
    pub(crate) fn of(dir: &Path) -> Self {
        let mut c = Self::default();
        match read_file(dir, "level")
            .and_then(|b| b.map(|b| parse("level", CompiledLevel::read(&b))).transpose())
        {
            Ok(Some(l)) => c.take_level(&l),
            Ok(None) => {}
            Err(e) => c.problems.push(e),
        }
        match read_file(dir, "level.cform")
            .and_then(|b| b.map(|b| parse("level.cform", CForm::read(&b))).transpose())
        {
            Ok(Some(f)) => c.collision = Some((f.verts.len(), f.tris.len(), f.aabb)),
            Ok(None) => {}
            Err(e) => c.problems.push(e),
        }
        match read_file(dir, "level.ai")
            .and_then(|b| b.map(|b| parse("level.ai", LevelGraph::read(&b))).transpose())
        {
            Ok(Some(g)) => c.ai_nodes = Some(g.nodes.len()),
            Ok(None) => {}
            Err(e) => c.problems.push(e),
        }
        match read_graph(dir) {
            Ok(Some(g)) => c.graph = Some((g.vertices.len(), g.edges.len())),
            Ok(None) => {}
            Err(e) => c.problems.push(e),
        }
        c.lightmap_pages = lightmap_files(dir).len();
        c
    }

    fn take_level(&mut self, l: &CompiledLevel) {
        self.visuals = Some(l.visuals.len());
        for v in &l.visuals {
            *self.kinds.entry(v.kind.name()).or_default() += 1;
            // a model's reference shares the model's geometry: only the visuals that own faces
            if let (Some(g), true) =
                (&v.geometry, matches!(v.kind, VisualKind::Normal | VisualKind::Progressive))
            {
                self.faces += g.faces() as usize;
                self.vertices += g.vertex_count as usize;
            }
        }
        self.shaders = l.shaders.len();
        self.sectors = l.sectors.len();
        self.portals = l.portals.len();
        self.lights = l.lights.len();
        self.glows = l.glows.len();
        self.quality = Some(l.quality);
        self.aabb = Some(l.aabb());
    }

    pub(crate) fn json(&self) -> Value {
        let mut o = serde_json::Map::new();
        if let Some(n) = self.visuals {
            o.insert("visuals".into(), json!({ "total": n, "by_kind": self.kinds }));
            o.insert("faces".into(), json!(self.faces));
            o.insert("vertices".into(), json!(self.vertices));
            o.insert("shaders".into(), json!(self.shaders));
            o.insert("sectors".into(), json!(self.sectors));
            o.insert("portals".into(), json!(self.portals));
            o.insert("lights".into(), json!(self.lights));
            o.insert("glows".into(), json!(self.glows));
        }
        if let Some(b) = &self.aabb {
            o.insert("box".into(), aabb(b));
        }
        if let Some((v, t, b)) = &self.collision {
            o.insert("collision".into(), json!({ "vertices": v, "triangles": t, "box": aabb(b) }));
        }
        o.insert("lightmap_pages".into(), json!(self.lightmap_pages));
        if let Some(n) = self.ai_nodes {
            o.insert("ai_nodes".into(), json!(n));
        }
        if let Some((v, e)) = self.graph {
            o.insert("graph_vertices".into(), json!(v));
            o.insert("graph_edges".into(), json!(e));
        }
        if !self.problems.is_empty() {
            o.insert("unreadable".into(), json!(self.problems));
        }
        Value::Object(o)
    }
}

fn read_graph(dir: &Path) -> Result<Option<gz_fmt_spawn::GameGraph>, String> {
    let Some(bytes) = read_file(dir, "level.graph")? else { return Ok(None) };
    let buf: Arc<[u8]> = bytes.into();
    parse("level.graph", gz_fmt_spawn::GameGraph::parse_file(&buf)).map(Some)
}

// ------------------------------------------------------------------------------------- the parts

fn read_part(dir: &Path, pieces: &Path, q: &Query) -> Result<Value, String> {
    if q.part == Part::Pieces {
        let mut out = pieces_part(pieces, q);
        if let Value::Object(o) = &mut out {
            o.insert("folder".into(), json!(show(pieces)));
            o.insert("part".into(), json!(q.part.name()));
        }
        return Ok(out);
    }
    if !dir.is_dir() {
        return Err(format!("{} is no folder.", show(dir)));
    }
    if !LEVEL_MARKERS.iter().any(|m| dir.join(m).is_file()) {
        return Err(format!(
            "{} is no level folder: it holds none of the files a level has (level, level.cform, level.ai, build.prj, ...).",
            show(dir)
        ));
    }
    let mut out = match q.part {
        Part::Summary => summary(dir, pieces, q)?,
        Part::Files => {
            let files = list_files(dir);
            page(files, q, "files", |(name, size)| json!({ "name": name, "size": size }))
        }
        Part::Visuals => with_level(dir, q, |l| Ok(visuals(l, q)))?,
        Part::Sectors => with_level(dir, q, |l| Ok(sectors(l, q)))?,
        Part::Portals => with_level(dir, q, |l| Ok(portals(l, q)))?,
        Part::Lights => with_level(dir, q, |l| Ok(lights(l, q)))?,
        Part::Lightmaps => lightmaps(dir, q)?,
        Part::Collision => collision(dir, q)?,
        Part::AiMap => ai_map(dir, q)?,
        Part::Graph => graph(dir, q)?,
        Part::Pieces => Value::Null,
    };
    if let Value::Object(o) = &mut out {
        o.insert("folder".into(), json!(show(dir)));
        o.entry("part").or_insert(json!(q.part.name()));
    }
    Ok(out)
}

fn with_level(
    dir: &Path,
    q: &Query,
    f: impl FnOnce(&CompiledLevel) -> Result<Value, String>,
) -> Result<Value, String> {
    match read_file(dir, "level")? {
        None => Ok(missing(q.part, "level")),
        Some(bytes) => f(&parse("level", CompiledLevel::read(&bytes))?),
    }
}

fn summary(dir: &Path, pieces: &Path, _q: &Query) -> Result<Value, String> {
    let counts = Counts::of(dir);
    let files = list_files(dir);
    let top: Vec<Value> = files
        .iter()
        .filter(|(n, _)| !n.contains('/'))
        .map(|(n, s)| json!({ "name": n, "size": s }))
        .collect();
    let mut folders: BTreeMap<&str, (usize, u64)> = BTreeMap::new();
    for (n, s) in &files {
        if let Some((folder, _)) = n.split_once('/') {
            let e = folders.entry(folder).or_default();
            e.0 += 1;
            e.1 += s;
        }
    }
    let folders: Vec<Value> =
        folders.into_iter().map(|(n, (c, s))| json!({ "name": n, "files": c, "bytes": s })).collect();
    let mut out = json!({
        "part": "summary",
        "counts": counts.json(),
        "files": top,
        "folders": folders,
    });
    let found = piece_folders(pieces);
    if !found.is_empty() {
        out["pieces"] = json!(found.iter().map(|p| piece_row(p)).collect::<Vec<_>>());
    }
    match read_file(dir, crate::report::FILE) {
        Ok(Some(bytes)) => {
            out["report"] = crate::report::read_json(&String::from_utf8_lossy(&bytes)).map_or(
                json!({ "unreadable": "build.report is not Lua data this editor can read." }),
                |mut r| {
                    // the lists the summary carries itself
                    if let Value::Object(o) = &mut r {
                        o.remove("files");
                        o.remove("counts");
                    }
                    r
                },
            );
        }
        Ok(None) => out["report"] = Value::Null,
        Err(e) => out["report"] = json!({ "unreadable": e }),
    }
    Ok(out)
}

fn visual_row(l: &CompiledLevel, v: &Visual) -> Value {
    let mut o = serde_json::Map::new();
    o.insert("index".into(), json!(v.index));
    o.insert("kind".into(), json!(v.kind.name()));
    o.insert("shader".into(), json!(v.shader));
    if let Some(s) = l.shader_of(v.shader) {
        o.insert("shader_name".into(), json!(s.shader));
        o.insert("texture".into(), json!(s.texture));
        o.insert("lightmap".into(), s.lightmap.map_or(Value::Null, |n| json!(n)));
    }
    o.insert("box".into(), aabb(&v.aabb));
    o.insert("sphere".into(), json!([r3(v.center.x), r3(v.center.y), r3(v.center.z), r3(v.radius)]));
    if let Some(g) = &v.geometry {
        o.insert("vertices".into(), json!(g.vertex_count));
        o.insert("faces".into(), json!(g.faces()));
        o.insert(
            "buffers".into(),
            json!({ "vertex": g.vertex_buffer, "vertex_start": g.vertex_start, "index": g.index_buffer, "index_start": g.index_start }),
        );
    }
    if !v.children.is_empty() {
        o.insert("children".into(), json!(v.children.len()));
        let ids: Vec<u32> = v.children.iter().copied().take(32).collect();
        o.insert("child_ids".into(), json!(ids));
    }
    if let Some(m) = &v.transform {
        o.insert("position".into(), json!([r3(m[3][0]), r3(m[3][1]), r3(m[3][2])]));
    }
    if v.lod {
        o.insert("lod".into(), json!(true));
    }
    if v.sliding {
        o.insert("sliding_windows".into(), json!(true));
    }
    o.insert("sector".into(), v.sector.map_or(Value::Null, |s| json!(s)));
    Value::Object(o)
}

fn meets(a: &Aabb, b: &Aabb) -> bool {
    a.min.cmple(b.max).all() && b.min.cmple(a.max).all()
}

fn visuals(l: &CompiledLevel, q: &Query) -> Value {
    let rows = l.visuals.iter().filter(|v| {
        q.bounds.as_ref().is_none_or(|b| meets(&v.aabb, b))
            && q.sector.is_none_or(|s| v.sector.is_some_and(|x| u32::from(x) == s))
            && q.shader.as_ref().is_none_or(|text| {
                l.shaders.get(usize::from(v.shader)).is_some_and(|s| s.to_lowercase().contains(text.as_str()))
            })
    });
    page(rows, q, "visuals", |v| visual_row(l, v))
}

fn sectors(l: &CompiledLevel, q: &Query) -> Value {
    let mut under: BTreeMap<u16, usize> = BTreeMap::new();
    for v in &l.visuals {
        if let Some(s) = v.sector {
            *under.entry(s).or_default() += 1;
        }
    }
    page(l.sectors.iter(), q, "sectors", |s| {
        let root = l.visuals.iter().find(|v| v.index == s.root);
        json!({
            "index": s.index,
            "root": s.root,
            "portals": s.portals,
            "visuals": under.get(&(s.index as u16)).copied().unwrap_or(0),
            "box": root.map_or(Value::Null, |v| aabb(&v.aabb)),
        })
    })
}

fn portals(l: &CompiledLevel, q: &Query) -> Value {
    page(l.portals.iter().enumerate(), q, "portals", |(i, p)| {
        let b = Aabb::from_points(&p.vertices);
        json!({
            "index": i,
            "front": p.front,
            "back": p.back,
            "vertices": p.vertices.iter().map(|v| vec3(*v)).collect::<Vec<_>>(),
            "box": aabb(&b),
        })
    })
}

fn lights(l: &CompiledLevel, q: &Query) -> Value {
    let mut out = page(l.lights.iter().enumerate(), q, "lights", |(i, x)| {
        let kind = match x.kind {
            1 => "point",
            2 => "spot",
            3 => "directional",
            _ => "other",
        };
        json!({
            "index": i,
            "controller": x.controller,
            "kind": kind,
            "position": vec3(x.position),
            "direction": vec3(x.direction),
            "range": r3(x.range),
            "color": [r3(x.diffuse[0]), r3(x.diffuse[1]), r3(x.diffuse[2])],
        })
    });
    out["glows"] = json!({
        "total": l.glows.len(),
        "rows": l.glows.iter().skip(q.offset).take(q.limit).map(|g| {
            json!({ "position": vec3(g.position), "size": r3(g.size), "shader": g.shader })
        }).collect::<Vec<_>>(),
    });
    out
}

fn lightmaps(dir: &Path, q: &Query) -> Result<Value, String> {
    let files = lightmap_files(dir);
    // how many visuals name each page; the level may be absent
    let mut used: BTreeMap<u32, usize> = BTreeMap::new();
    let level = match read_file(dir, "level")? {
        Some(b) => Some(parse("level", CompiledLevel::read(&b))?),
        None => None,
    };
    if let Some(l) = &level {
        for v in &l.visuals {
            if let Some(n) = l.shader_of(v.shader).and_then(|s| s.lightmap) {
                *used.entry(n).or_default() += 1;
            }
        }
    }
    let mut out = page(files, q, "pages", |(n, list)| {
        let maps: Vec<Value> = list
            .iter()
            .map(|(name, info)| match info {
                Some((w, h, mips, bytes)) => {
                    json!({ "file": name, "width": w, "height": h, "mip_levels": mips, "bytes": bytes })
                }
                None => json!({ "file": name, "unreadable": "not a DDS file" }),
            })
            .collect();
        json!({ "page": n, "files": maps, "visuals": used.get(&n).copied().unwrap_or(0) })
    });
    if level.is_none() {
        out["note"] = json!("The level file is missing, so the visuals per page are not counted.");
    }
    Ok(out)
}

/// The rows of a count table, the biggest first.
fn ranked(counts: BTreeMap<u16, usize>, key: &str, q: &Query) -> Value {
    let mut rows: Vec<(u16, usize)> = counts.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    page(rows, q, "rows", |(id, n)| json!({ key: id, "triangles": n }))
}

fn collision(dir: &Path, q: &Query) -> Result<Value, String> {
    let Some(bytes) = read_file(dir, "level.cform")? else { return Ok(missing(q.part, "level.cform")) };
    let f = parse("level.cform", CForm::read(&bytes))?;
    let (mut by_material, mut by_sector) = (BTreeMap::new(), BTreeMap::new());
    let (mut no_shadows, mut no_marks) = (0usize, 0usize);
    for t in f.tris.iter() {
        *by_material.entry(t.material()).or_default() += 1usize;
        *by_sector.entry(t.sector()).or_default() += 1usize;
        no_shadows += usize::from(t.suppress_shadows());
        no_marks += usize::from(t.suppress_wallmarks());
    }
    Ok(json!({
        "version": f.version,
        "vertices": f.verts.len(),
        "triangles": f.tris.len(),
        "box": aabb(&f.aabb),
        "suppress_shadows": no_shadows,
        "suppress_wallmarks": no_marks,
        "materials": ranked(by_material, "material", q),
        "sectors": ranked(by_sector, "sector", q),
    }))
}

fn ai_map(dir: &Path, q: &Query) -> Result<Value, String> {
    let Some(bytes) = read_file(dir, "level.ai")? else { return Ok(missing(q.part, "level.ai")) };
    let g = parse("level.ai", LevelGraph::read(&bytes))?;
    let h = &g.header;
    Ok(json!({
        "version": h.version,
        "guid": hex(&h.guid),
        "nodes": g.nodes.len(),
        "cell_size": r3(h.cell_size),
        "height_range": r3(h.factor_y),
        "box": aabb(&h.aabb),
        "columns": h.column_length(),
        "rows": h.row_length(),
    }))
}

/// The piece folders under `pieces`, by name: each that holds a `piece`.
pub(crate) fn piece_folders(pieces: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(pieces) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .map(|e| e.path())
        .filter(|p| p.join("piece").is_file())
        .collect();
    out.sort();
    out
}

/// The keys of a piece's `piece.ltx` (`[piece]`), lower case.
fn piece_manifest(dir: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Ok(bytes) = fs::read(dir.join("piece.ltx")) else { return out };
    let text = String::from_utf8_lossy(&bytes);
    let mut inside = false;
    for line in text.lines() {
        let line = line.split(';').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            inside = line.eq_ignore_ascii_case("[piece]");
        } else if inside && let Some((k, v)) = line.split_once('=') {
            out.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
        }
    }
    out
}

/// One piece as a row: its name, the manifest's box, sector and streaming, what its files hold,
/// and what the game would refuse it for.
pub(crate) fn piece_row(dir: &Path) -> Value {
    let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let m = piece_manifest(dir);
    let mut o = serde_json::Map::new();
    o.insert("name".into(), json!(name));
    let numbers: Vec<f64> = m
        .get("box")
        .map(|b| b.split(',').filter_map(|x| x.trim().parse().ok()).collect())
        .unwrap_or_default();
    o.insert("box".into(), if numbers.len() == 6 { json!(numbers) } else { Value::Null });
    o.insert("sector".into(), json!(m.get("sector").cloned().unwrap_or_else(|| "auto".into())));
    o.insert("stream".into(), json!(m.get("stream").is_none_or(|s| !s.eq_ignore_ascii_case("false"))));
    if let Some(v) = m.get("version") {
        o.insert("version".into(), json!(v));
    }
    let mut problems: Vec<String> = Vec::new();
    match fs::read(dir.join("piece"))
        .map_err(|e| e.to_string())
        .and_then(|b| CompiledLevel::read_piece(&b).map_err(|e| e.to_string()))
    {
        Ok(p) => {
            let leaves =
                p.visuals.iter().filter(|v| matches!(v.kind, VisualKind::Normal | VisualKind::Progressive));
            let (faces, vertices) = leaves.fold((0usize, 0usize), |(f, v), x| {
                x.geometry.map_or((f, v), |g| (f + g.faces() as usize, v + g.vertex_count as usize))
            });
            o.insert("visuals".into(), json!(p.visuals.len()));
            o.insert("roots".into(), json!(p.roots.len()));
            o.insert("shaders".into(), json!(p.shaders.len()));
            o.insert("faces".into(), json!(faces));
            o.insert("vertices".into(), json!(vertices));
            let geom = fs::read(dir.join("piece.geom"))
                .map_err(|e| e.to_string())
                .and_then(|b| gz_fmt_level::GeomTables::read(&b).map_err(|e| e.to_string()));
            let fast = match fs::read(dir.join("piece.geomx")) {
                Ok(b) => Some(gz_fmt_level::GeomTables::read(&b).map_err(|e| e.to_string())),
                Err(_) => None,
            };
            match (geom, fast.transpose()) {
                (Ok(g), Ok(f)) => problems.extend(p.piece_problems(&g, f.as_ref())),
                (Err(e), _) | (_, Err(e)) => problems.push(format!("piece.geom could not be read: {e}")),
            }
        }
        Err(e) => problems.push(format!("piece could not be read: {e}")),
    }
    o.insert("lightmap_pages".into(), json!(lightmap_files(dir).len()));
    o.insert("files".into(), json!(list_files(dir).len()));
    if !m.contains_key("box") {
        problems.push("piece.ltx names no box.".to_owned());
    }
    o.insert("problems".into(), json!(problems));
    Value::Object(o)
}

fn pieces_part(pieces: &Path, q: &Query) -> Value {
    let found = piece_folders(pieces);
    let mut out = page(found.iter(), q, "pieces", |p| piece_row(p));
    if found.is_empty() {
        out["note"] = json!(
            "The level has no piece: a module's piece of a level of the game, or a level's streaming cells, are what the level build makes into this folder."
        );
    }
    out
}

fn graph(dir: &Path, q: &Query) -> Result<Value, String> {
    let Some(g) = read_graph(dir)? else { return Ok(missing(q.part, "level.graph")) };
    Ok(json!({
        "version": g.version,
        "guid": hex(&g.guid),
        "vertices": g.vertices.len(),
        "edges": g.edges.len(),
        "death_points": g.death_points.len(),
        "levels": g.levels.iter().map(|l| l.name.to_string()).collect::<Vec<_>>(),
    }))
}

/// A hand-built compiled level for the tests of this crate: two leaves under a hierarchy, one
/// sector, one portal, one light, one glow.
#[cfg(test)]
pub(crate) mod sample {
    use glam::Vec3;
    use gz_chunk::Writer;

    fn header(w: &mut Writer, ty: u8, shader: u16, min: Vec3, max: Vec3) {
        w.chunk(1, |w| {
            w.w_u8(4);
            w.w_u8(ty);
            w.w_u16(shader);
            w.w_vec3(min);
            w.w_vec3(max);
            w.w_vec3((min + max) * 0.5);
            w.w_f32(1.0);
        });
    }

    fn leaf(w: &mut Writer, shader: u16, min: Vec3, max: Vec3, verts: u32, indices: u32) {
        w.chunk(21, |w| {
            for v in [0, 0, verts, 0, 0, indices] {
                w.w_u32(v);
            }
        });
        header(w, 0, shader, min, max);
    }

    /// A piece: two leaves in one vertex and one index buffer of its own, both roots, the first
    /// hung on sector 3, the second on the sector at its centre.
    pub(crate) fn piece() -> (Vec<u8>, Vec<u8>) {
        let mut piece = Writer::new();
        piece.chunk(1, |w| {
            w.w_u16(14);
            w.w_u16(1);
        });
        let mut visuals = Writer::new();
        visuals.chunk(0, |w| leaf(w, 1, Vec3::ZERO, Vec3::ONE, 4, 6));
        visuals.chunk(1, |w| {
            w.chunk(21, |w| {
                for v in [0, 4, 4, 0, 6, 6] {
                    w.w_u32(v);
                }
            });
            header(w, 0, 1, Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 1.0));
        });
        piece.chunk(3, |w| w.w(visuals.bytes()));
        piece.chunk(2, |w| {
            w.w_u32(2);
            w.w_stringz("");
            w.w_stringz(r"def_shaders\lmap/wall\wall_a,lmap#1_1,lmap#1_2");
        });
        piece.chunk(32, |w| {
            w.w_u32(2);
            for (v, s) in [(0u32, 3u16), (1, 0xFFFF)] {
                w.w_u32(v);
                w.w_u16(s);
                w.w_u16(0);
            }
        });
        let mut geom = Writer::new();
        geom.chunk(9, |w| {
            w.w_u32(1);
            // one float3 position and the end mark
            w.w(&[0, 0, 0, 0, 2, 0, 0, 0, 0xFF, 0, 0, 0, 17, 0, 0, 0]);
            w.w_u32(8);
            w.w(&[0u8; 8 * 12]);
        });
        geom.chunk(10, |w| {
            w.w_u32(1);
            w.w_u32(12);
            w.w(&[0u8; 24]);
        });
        geom.chunk(11, |w| w.w_u32(0));
        (piece.into_bytes(), geom.into_bytes())
    }

    pub(crate) fn level() -> Vec<u8> {
        let mut level = Writer::new();
        level.chunk(1, |w| {
            w.w_u16(14);
            w.w_u16(2);
        });
        level.chunk(2, |w| {
            w.w_u32(3);
            w.w_stringz("");
            w.w_stringz(r"def_shaders\lmap/wall\wall_a,lmap#1");
            w.w_stringz(r"def_shaders\vert/floor");
        });
        let mut visuals = Writer::new();
        visuals.chunk(0, |w| leaf(w, 1, Vec3::ZERO, Vec3::ONE, 4, 6));
        visuals.chunk(1, |w| leaf(w, 2, Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 1.0), 8, 12));
        visuals.chunk(2, |w| {
            header(w, 1, 0, Vec3::ZERO, Vec3::new(2.0, 1.0, 1.0));
            w.chunk(10, |w| {
                w.w_u32(2);
                w.w_u32(0);
                w.w_u32(1);
            });
        });
        level.chunk(3, |w| w.w(visuals.bytes()));
        level.chunk(4, |w| {
            w.w_u16(0);
            w.w_u16(1);
            for k in 0..6 {
                w.w_vec3(Vec3::splat(k as f32));
            }
            w.w_u32(4);
        });
        level.chunk(6, |w| {
            w.w_u32(7);
            w.w_u32(1);
            for _ in 0..12 {
                w.w_f32(0.5);
            }
            w.w_vec3(Vec3::new(1.0, 2.0, 3.0));
            w.w_vec3(Vec3::Y);
            for v in [9.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0] {
                w.w_f32(v);
            }
        });
        level.chunk(7, |w| {
            w.w_vec3(Vec3::new(5.0, 5.0, 5.0));
            w.w_f32(0.25);
            w.w_u16(2);
        });
        let mut sectors = Writer::new();
        sectors.chunk(0, |w| {
            w.chunk(2, |w| w.w_u32(2));
            w.chunk(1, |w| w.w_u16(0));
        });
        level.chunk(8, |w| w.w(sectors.bytes()));
        level.into_bytes()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use gz_editor::{Editor, EditorOptions, Log, Origin};
    use gz_task::{ExecOptions, Runtime};

    fn editor() -> (Runtime, Arc<Editor>) {
        let rt = Runtime::start(ExecOptions { async_threads: 2, cpu_threads: 1, io_threads: 2 }).unwrap();
        let (log, _guard) = Log::start(None, false).unwrap();
        let mut reg = Registry::new();
        register(&mut reg);
        reg.install_help();
        let ed = Editor::new(EditorOptions::default(), rt.exec(), log, reg);
        (rt, ed)
    }

    fn call(rt: &Runtime, ed: &Arc<Editor>, args: Value) -> Value {
        rt.block_on(ed.call("level.inspect", args.as_object().cloned().unwrap(), Origin::Mcp))
    }

    fn level_folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("level"), sample::level()).unwrap();
        dir
    }

    #[test]
    fn the_parts_answer_from_a_level_without_a_project() {
        let (rt, ed) = editor();
        let dir = level_folder();
        let folder = dir.path().to_string_lossy().into_owned();
        let r = call(&rt, &ed, json!({ "folder": folder }));
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!(r["counts"]["visuals"]["total"], 3);
        assert_eq!(r["counts"]["visuals"]["by_kind"]["normal"], 2);
        assert_eq!(r["counts"]["faces"], 6);
        assert_eq!(r["counts"]["sectors"], 1);
        assert_eq!(r["report"], Value::Null);

        // paging, and the answer says how many there are
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "visuals", "limit": 1, "offset": 1 }));
        assert_eq!((r["total"].clone(), r["visuals"].as_array().unwrap().len()), (json!(3), 1), "{r}");
        assert_eq!(r["visuals"][0]["index"], 1);
        assert_eq!(r["visuals"][0]["texture"], "floor");
        // the shader filter, the box filter, the sector filter
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "visuals", "shader": "LMAP" }));
        assert_eq!(r["total"], 1);
        assert_eq!(r["visuals"][0]["lightmap"], 1);
        assert_eq!(r["visuals"][0]["texture"], "wall\\wall_a");
        let r = call(
            &rt,
            &ed,
            json!({ "folder": folder, "part": "visuals", "box": [1.5, 0.0, 0.0, 3.0, 1.0, 1.0] }),
        );
        let ids: Vec<u64> =
            r["visuals"].as_array().unwrap().iter().map(|v| v["index"].as_u64().unwrap()).collect();
        assert_eq!(ids, [1, 2], "{r}");
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "visuals", "sector": 0 }));
        assert_eq!(r["total"], 3);
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "visuals", "sector": 4 }));
        assert_eq!(r["total"], 0);
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "visuals", "box": [1.0, 2.0] }));
        assert_eq!(r["ok"], false, "{r}");

        let r = call(&rt, &ed, json!({ "folder": folder, "part": "sectors" }));
        assert_eq!(
            (r["sectors"][0]["root"].clone(), r["sectors"][0]["visuals"].clone()),
            (json!(2), json!(3))
        );
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "portals" }));
        assert_eq!(r["portals"][0]["vertices"].as_array().unwrap().len(), 4);
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "lights" }));
        assert_eq!((r["lights"][0]["kind"].clone(), r["glows"]["total"].clone()), (json!("point"), json!(1)));
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "files" }));
        assert_eq!(r["files"][0]["name"], "level");
    }

    #[test]
    fn the_pieces_of_a_level_are_read_with_what_the_game_would_refuse() {
        let (rt, ed) = editor();
        let dir = level_folder();
        let folder = dir.path().to_string_lossy().into_owned();
        // no piece yet: said, and the summary carries none
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "pieces" }));
        assert_eq!((r["ok"].clone(), r["total"].clone()), (json!(true), json!(0)), "{r}");
        assert!(r["note"].as_str().unwrap().contains("no piece"));
        assert!(call(&rt, &ed, json!({ "folder": folder }))["pieces"].is_null());

        let (piece, geom) = sample::piece();
        let main = dir.path().join("pieces").join("main");
        fs::create_dir_all(&main).unwrap();
        fs::write(main.join("piece"), &piece).unwrap();
        fs::write(main.join("piece.geom"), &geom).unwrap();
        fs::write(main.join("lmap#1_1.dds"), b"DDS ").unwrap();
        fs::write(main.join("lmap#1_2.dds"), b"DDS ").unwrap();
        fs::write(
            main.join("piece.ltx"),
            "[piece]\r\nversion = 1\r\nbox     = 0.000, 0.000, 0.000, 2.000, 1.000, 1.000\r\nsector  = auto\r\nstream  = true\r\n",
        )
        .unwrap();
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "pieces" }));
        assert_eq!(r["total"], 1, "{r}");
        let p = &r["pieces"][0];
        assert_eq!(
            (p["name"].clone(), p["stream"].clone(), p["sector"].clone()),
            (json!("main"), json!(true), json!("auto"))
        );
        assert_eq!(p["box"], json!([0.0, 0.0, 0.0, 2.0, 1.0, 1.0]));
        assert_eq!((p["visuals"].clone(), p["roots"].clone()), (json!(2), json!(2)));
        assert_eq!(
            (p["vertices"].clone(), p["faces"].clone(), p["lightmap_pages"].clone()),
            (json!(8), json!(4), json!(1))
        );
        assert_eq!(p["problems"], json!([]), "{p}");
        // in the summary too
        let s = call(&rt, &ed, json!({ "folder": folder }));
        assert_eq!(s["pieces"][0]["name"], "main", "{s}");
        // a piece whose visual names a range past its buffers is one the game refuses
        let mut short = geom.clone();
        let at = short.len() - 4 - 8 - 24 - 4;
        short[at..at + 4].copy_from_slice(&10u32.to_le_bytes());
        fs::write(main.join("piece.geom"), &short).unwrap();
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "pieces" }));
        let problems = r["pieces"][0]["problems"].as_array().unwrap();
        assert!(problems.iter().any(|x| x.as_str().unwrap().contains("outside its buffers")), "{r}");
        // the reader of a compiled level reads the piece: roots, their sectors, nothing of a level's
        let l = CompiledLevel::read_piece(&piece).unwrap();
        assert_eq!(l.roots.len(), 2);
        assert_eq!((l.visuals[0].sector, l.visuals[1].sector), (Some(3), None));
        assert!(CompiledLevel::read_piece(&sample::level()).is_err(), "a level is no piece");
    }

    #[test]
    fn a_file_that_is_not_there_is_said_plainly() {
        let (rt, ed) = editor();
        let dir = level_folder();
        let folder = dir.path().to_string_lossy().into_owned();
        for (part, file) in [("collision", "level.cform"), ("ai_map", "level.ai"), ("graph", "level.graph")] {
            let r = call(&rt, &ed, json!({ "folder": folder, "part": part }));
            assert_eq!(r["ok"], true, "{part}: {r}");
            assert_eq!(r["missing"], file);
        }
        // the lightmaps need no level file for the pages, and say so
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "lightmaps" }));
        assert_eq!(r["total"], 0, "{r}");
    }

    #[test]
    fn a_folder_that_is_no_level_is_refused() {
        let (rt, ed) = editor();
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("notes.txt"), "x").unwrap();
        let r = call(&rt, &ed, json!({ "folder": dir.path().to_string_lossy() }));
        assert_eq!(r["ok"], false);
        assert!(r["error"].as_str().unwrap().contains("no level folder"), "{r}");
        let r = call(&rt, &ed, json!({ "folder": dir.path().join("nope").to_string_lossy() }));
        assert!(r["error"].as_str().unwrap().contains("no folder"), "{r}");
        // neither a project nor a folder
        let r = call(&rt, &ed, json!({ "level": "l01" }));
        assert_eq!(r["ok"], false);
    }

    #[test]
    fn a_damaged_level_file_is_a_sentence_and_the_summary_goes_on() {
        let (rt, ed) = editor();
        let dir = level_folder();
        let bytes = fs::read(dir.path().join("level")).unwrap();
        fs::write(dir.path().join("level"), &bytes[..bytes.len() - 3]).unwrap();
        let folder = dir.path().to_string_lossy().into_owned();
        let r = call(&rt, &ed, json!({ "folder": folder, "part": "visuals" }));
        assert_eq!(r["ok"], false);
        assert!(r["error"].as_str().unwrap().contains("level could not be read"), "{r}");
        let r = call(&rt, &ed, json!({ "folder": folder }));
        assert_eq!(r["ok"], true);
        assert!(r["counts"]["unreadable"][0].as_str().unwrap().contains("level"), "{r}");
    }
}
