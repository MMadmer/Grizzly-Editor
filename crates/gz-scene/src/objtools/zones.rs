//! The visibility cells of a level - sectors and the portals between them - as the build makes
//! them and as an author may draw them by hand.
//!
//! The build needs no hand-made cell ([`complete`]): a level without sectors is one cell; a
//! level with some gets the default cell for every static mesh no sector holds, and the portals
//! between the cells that have none are computed from the cells' faces. What the author made is
//! kept as it is. `scene.zones` lists the cells, makes and fills them by hand, computes the
//! portals into the scene, checks them and removes them.

use std::sync::Arc;

use gz_base::{Finding, Origin, SmolStr};
use gz_editor::{CmdError, Editor, JsonSchema, Registry, Reply, Spec, Value, json};
use gz_fmt_xr::ShadersXrlc;
use rustc_hash::FxHashSet;
use serde::Deserialize;

use super::portal;
use super::sector::{self, DEFAULT_SECTOR};
use super::{Batch, NameRule, Outcome, commit, commit_job, set_state};
use crate::class::ObjClass;
use crate::library::Library;
use crate::model::{ObjData, ObjId, Object, SceneState};

/// The section of the scene's reference (`docs/assets/SCENE.md`) on what the build does with
/// `zones.lua`.
pub const REFERENCE: &str = "## Zones: what the build does with `zones.lua`\n\n\
A level needs no hand-made zone. The build's geometry step completes what the scene holds and compiles the \
completed copy, never writing it back:\n\n\
- a scene without sectors is one cell - nothing is added;\n\
- a scene with sectors gets the **default sector** (`sector_default`, `default = true`) for every static mesh \
no sector's `items` hold - an existing `sector_default` is filled, else one is made;\n\
- between the sectors no portal joins, **portals are computed** from the sectors' faces; a pair of sectors the \
author joined by hand keeps the author's portals.\n\n\
`zones.lua` therefore holds only what the author made by hand (the Zones mode, `scene.zones`): a sector's \
`items` name a scene object and one mesh of its library object, compared without case; a portal's `front` and \
`back` name sectors; its `vertices` are its outline in order, at least three. `scene.zones action=compute` keeps \
in the scene what the build would add, and `action=list` answers `uncovered_meshes`, what the build would put \
into the default sector.\n\n";

/// What the build added to the scene's cells, and what it found.
#[derive(Clone, Debug, Default)]
pub struct Completion {
    /// The scene the build compiles: the author's cells and what the build added.
    pub state: SceneState,
    /// Meshes the build put into the default cell.
    pub default_meshes: usize,
    /// Portals the build computed.
    pub portals: usize,
    /// What the computation said, with the cells it is about.
    pub notes: Vec<(Finding, Vec<String>)>,
}

/// A computation was asked to stop.
pub struct Stopped;

fn pair(a: &str, b: &str) -> (String, String) {
    let (a, b) = (a.to_lowercase(), b.to_lowercase());
    if a <= b { (a, b) } else { (b, a) }
}

fn portal_name(state: &SceneState, n: &mut u32) -> String {
    loop {
        let name = format!("portal_auto_{:02}", *n);
        *n += 1;
        if !state.name_used(&name) {
            return name;
        }
    }
}

/// The scene's cells completed for a build: nothing changes in a scene without sectors (the whole
/// level is one cell); otherwise the meshes no sector holds go into the default cell, and between
/// the cells no portal joins, portals are computed. Pure; the library objects of the scene must
/// be loaded.
pub fn complete(
    state: &SceneState,
    lib: &Library,
    stop: &(dyn Fn() -> bool + Sync),
) -> Result<Completion, Stopped> {
    let mut out = Completion { state: state.clone(), ..Completion::default() };
    if state.count(ObjClass::Sector) == 0 {
        return Ok(out);
    }
    let unused = sector::unused_meshes(state, lib);
    if !unused.is_empty() {
        let items: Vec<(SmolStr, SmolStr)> = unused
            .iter()
            .filter_map(|(o, lo, m)| lo.object.meshes.get(*m).map(|mesh| (o.name.clone(), mesh.name.clone())))
            .collect();
        out.default_meshes = items.len();
        match out.state.find(ObjClass::Sector, DEFAULT_SECTOR).cloned() {
            Some(existing) => {
                let mut s = (*existing).clone();
                if let ObjData::Sector(d) = &mut s.data {
                    d.items.extend(items);
                }
                out.state.replace(s);
            }
            None => {
                let mut s = sector::new_sector(items);
                s.name = SmolStr::new(DEFAULT_SECTOR);
                if let ObjData::Sector(d) = &mut s.data {
                    d.color = [1.0, 0.0, 0.0, 0.0];
                    d.default = true;
                }
                out.state.insert(s);
            }
        }
        out.notes.push((
            Finding::info(format!(
                "The build put {} no visibility cell holds into the default cell.",
                if out.default_meshes == 1 {
                    "1 mesh".to_owned()
                } else {
                    format!("{} meshes", out.default_meshes)
                }
            )),
            vec![DEFAULT_SECTOR.to_owned()],
        ));
    }
    if out.state.count(ObjClass::Sector) < 2 {
        return Ok(out);
    }
    let joined: FxHashSet<(String, String)> = out
        .state
        .of_class(ObjClass::Portal)
        .filter_map(|p| match &p.data {
            ObjData::Portal(d) => Some(pair(&d.front, &d.back)),
            _ => None,
        })
        .collect();
    let sectors: Vec<ObjId> = out.state.of_class(ObjClass::Sector).map(|o| o.id).collect();
    let snapshot = out.state.clone();
    let mut n = 0u32;
    let mut names = || portal_name(&snapshot, &mut n);
    let c = portal::compute(&out.state, lib, &sectors, &mut names, stop, &|_| {}).map_err(|_| Stopped)?;
    let mut k = 0u32;
    for p in c.portals {
        if joined.contains(&pair(&p.front, &p.back)) {
            continue;
        }
        let name = portal_name(&out.state, &mut k);
        out.state.insert(Object::new(ObjClass::Portal, &name, ObjData::Portal(p)));
        out.portals += 1;
    }
    for line in c.log {
        let cells: Vec<String> = line
            .split('\'')
            .skip(1)
            .step_by(2)
            .filter(|s| out.state.find(ObjClass::Sector, &s.to_lowercase()).is_some())
            .map(str::to_owned)
            .collect();
        out.notes.push((Finding::warning(line.replace('\n', " ")), cells));
    }
    Ok(out)
}

/// What `scene.zones` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ZoneAction {
    /// The cells and portals there are, and how many meshes no cell holds.
    #[default]
    List,
    /// A new cell of the static meshes of `objects` (left out: the selected objects).
    Create,
    /// The static meshes of `objects` join the cell `sector`, leaving any other; `inside` takes
    /// every static mesh inside the cell's volume too.
    Assign,
    /// What the build does, kept in the scene: the default cell for the meshes no cell holds and
    /// the portals between the cells (`objects`: only between these cells).
    Compute,
    /// Checks the cells as the build does.
    Validate,
    /// Removes cells or portals by name (a cell takes its portals with it).
    Remove,
    /// Turns portals round (`objects`: their names; left out: the selected ones).
    Invert,
    /// What a click in the Zones mode does.
    Mode,
}

/// What a click on a mesh does in the Zones mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ZoneClick {
    /// Adds the mesh to (or takes it out of) the selected cell.
    Edit,
    /// Makes a new cell of the mesh.
    NewFromMesh,
    /// Makes a new cell of each mesh of the object.
    NewFromObject,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ZonesParams {
    /// What to do; left out: list.
    #[serde(default)]
    action: ZoneAction,
    /// create, assign: the scene objects (left out: the selection); compute: cells; remove,
    /// invert: cells or portals.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// assign: the cell that takes the meshes.
    #[serde(default)]
    sector: Option<String>,
    /// create: the new cell's name.
    #[serde(default)]
    name: Option<String>,
    /// assign: every static mesh inside the cell's volume joins it too.
    #[serde(default)]
    inside: bool,
    /// mode: what a click on a mesh does.
    #[serde(default)]
    click: Option<ZoneClick>,
    /// mode: a click on a mesh adds it to the selected cell (add) or takes it out (remove).
    #[serde(default)]
    meshes: Option<String>,
    /// mode: a drag picks meshes by a box.
    #[serde(default)]
    box_pick: Option<bool>,
}

/// Registers `scene.zones`.
pub fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::write(
            "scene.zones",
            "The visibility cells and portals: list, create, assign, compute, validate, remove. The build makes what is missing by itself.",
        )
        .example("action=compute"),
        |ed, p: ZonesParams, o| async move { zones(ed, p, o).await },
    );
}

fn scene(ed: &Arc<Editor>) -> Result<Arc<crate::SceneService>, CmdError> {
    let svc = crate::SceneService::of(ed)?;
    if !svc.store.snapshot().valid {
        return Err(CmdError::not_found("No scene is open."));
    }
    Ok(svc)
}

async fn shaders(ed: &Arc<Editor>) -> Option<Arc<ShadersXrlc>> {
    let (bytes, _) = gz_game::read_game_or_bundled(ed, "shaders_xrlc.xr").await.ok()?;
    ShadersXrlc::read(&bytes).ok().map(Arc::new)
}

/// Scene objects by name, or the selected ones of a class.
fn named_or_selected(
    s: &SceneState,
    names: &Option<Vec<String>>,
    class: ObjClass,
) -> Result<Vec<ObjId>, CmdError> {
    match names {
        Some(list) => list
            .iter()
            .map(|n| {
                s.find(class, &n.to_lowercase()).map(|o| o.id).ok_or_else(|| {
                    CmdError::not_found(format!("there is no {} '{n}'", class.desc().to_lowercase()))
                })
            })
            .collect(),
        None => Ok(super::selected_of(s, class)),
    }
}

/// Every static mesh of the objects no other cell than `except` holds.
fn meshes_of(s: &SceneState, lib: &Library, ids: &[ObjId]) -> Vec<(SmolStr, SmolStr)> {
    let mut out = Vec::new();
    for id in ids {
        let Some(o) = s.get(*id) else { continue };
        let ObjData::SceneObject(d) = &o.data else { continue };
        let Some(lo) = lib.cached(&d.reference) else { continue };
        if !super::geom::is_static_any(&lo) {
            continue;
        }
        for m in &lo.object.meshes {
            out.push((o.name.clone(), m.name.clone()));
        }
    }
    out
}

fn list(s: &SceneState, lib: &Library) -> Value {
    let sectors: Vec<Value> = s.of_class(ObjClass::Sector).map(|o| sector::sector_json(s, lib, o)).collect();
    let portals: Vec<Value> = s
        .of_class(ObjClass::Portal)
        .filter_map(|o| match &o.data {
            ObjData::Portal(p) => Some(json!({
                "name": o.name.as_str(),
                "front": p.front.as_str(),
                "back": p.back.as_str(),
                "vertices": p.vertices.len(),
            })),
            _ => None,
        })
        .collect();
    let uncovered = if s.count(ObjClass::Sector) == 0 { 0 } else { sector::unused_meshes(s, lib).len() };
    json!({
        "sectors": sectors,
        "portals": portals,
        "uncovered_meshes": uncovered,
        "note": "The build makes the level one cell when it has none, puts the meshes no cell holds into the default cell and computes the portals the cells lack.",
    })
}

async fn zones(ed: Arc<Editor>, p: ZonesParams, o: Origin) -> Reply {
    let svc = scene(&ed)?;
    match p.action {
        ZoneAction::List => {
            let snap = svc.store.snapshot();
            svc.prefetch_library(&ed, &snap).await;
            let lib = svc.library.clone();
            Ok(ed.exec.cpu(move || list(&snap, &lib)).await)
        }
        ZoneAction::Create => {
            let snap = svc.store.snapshot();
            svc.prefetch_library(&ed, &snap).await;
            let ids = named_or_selected(&snap, &p.objects, ObjClass::SceneObject)?;
            if ids.is_empty() {
                return Err(CmdError::invalid("Name the objects of the new cell, or select them."));
            }
            let name = p.name.clone();
            commit(&ed, &svc, o, super::Ctl::default(), move |s, lib, _| {
                let held = sector::unused_meshes(s, lib);
                let free: FxHashSet<(String, String)> = held
                    .iter()
                    .filter_map(|(ob, lo, m)| {
                        lo.object.meshes.get(*m).map(|x| (ob.name.to_lowercase(), x.name.to_lowercase()))
                    })
                    .collect();
                let items: Vec<(SmolStr, SmolStr)> = meshes_of(s, lib, &ids)
                    .into_iter()
                    .filter(|(a, b)| free.contains(&(a.to_lowercase(), b.to_lowercase())))
                    .collect();
                if items.is_empty() {
                    return Err(CmdError::invalid(
                        "Every static mesh of these objects is in a cell already.",
                    ));
                }
                let n = items.len();
                let mut b = Batch::new("New Cell", s.serial);
                let rule = name.clone().map_or(NameRule::Gen(Some("sector".into())), NameRule::Fixed);
                b.add.push((sector::new_sector(items), rule));
                b.select_added = true;
                Ok((b, json!({ "meshes": n })))
            })
            .await
        }
        ZoneAction::Assign => {
            let snap = svc.store.snapshot();
            svc.prefetch_library(&ed, &snap).await;
            let cell =
                p.sector.clone().ok_or_else(|| CmdError::invalid("assign needs 'sector': the cell"))?;
            let cid = snap
                .find(ObjClass::Sector, &cell.to_lowercase())
                .map(|x| x.id)
                .ok_or_else(|| CmdError::not_found(format!("there is no cell '{cell}'")))?;
            let ids = if p.objects.is_none() && p.inside {
                Vec::new()
            } else {
                named_or_selected(&snap, &p.objects, ObjClass::SceneObject)?
            };
            let inside = p.inside;
            let out = commit(&ed, &svc, o, super::Ctl::default(), move |s, lib, _| assign(s, lib, cid, &ids))
                .await?;
            if !inside {
                return Ok(out);
            }
            let more = commit(&ed, &svc, o, super::Ctl::default(), move |s, lib, _| {
                sector::capture_volume(s, lib, cid)
            })
            .await?;
            Ok(
                json!({ "sector": cell, "moved": out.get("moved").cloned().unwrap_or(json!(0)), "captured": more.get("added").cloned().unwrap_or(json!(0)) }),
            )
        }
        ZoneAction::Compute => {
            let only = p.objects.clone();
            commit_job(&ed, &svc, o, "Computing the visibility cells", move |s, lib, ctl| {
                compute(s, lib, ctl, only.as_deref())
            })
            .await
        }
        ZoneAction::Validate => {
            let sh = shaders(&ed).await;
            let snap = svc.store.snapshot();
            svc.prefetch_library(&ed, &snap).await;
            let lib = svc.library.clone();
            let (valid, problems) = ed.exec.cpu(move || sector::validate(&snap, &lib, sh.as_deref())).await;
            let mut out = json!({ "valid": valid, "problems": problems });
            if !valid && svc.store.snapshot().count(ObjClass::Sector) > 0 {
                out["note"] = json!("The build puts meshes no cell holds into the default cell by itself.");
            }
            Ok(out)
        }
        ZoneAction::Remove => {
            let names = p
                .objects
                .clone()
                .ok_or_else(|| CmdError::invalid("remove needs 'objects': cells or portals"))?;
            commit(&ed, &svc, o, super::Ctl::default(), move |s, _, _| {
                let mut b = Batch::new("Remove Cells", s.serial);
                for n in &names {
                    let key = n.to_lowercase();
                    if let Some(c) = s.find(ObjClass::Sector, &key) {
                        b.remove.push(c.id);
                        b.remove.extend(sector::sector_portals(s, &c.name));
                    } else if let Some(pt) = s.find(ObjClass::Portal, &key) {
                        b.remove.push(pt.id);
                    } else {
                        return Err(CmdError::not_found(format!("there is no cell or portal '{n}'")));
                    }
                }
                b.remove.sort_unstable();
                b.remove.dedup();
                let n = b.remove.len();
                Ok((b, json!({ "removed": n })))
            })
            .await
        }
        ZoneAction::Invert => {
            let names = p.objects.clone();
            commit(&ed, &svc, o, super::Ctl::default(), move |s, _, _| {
                let ids = named_or_selected(s, &names, ObjClass::Portal)?;
                let mut b = Batch::new("Invert Portals", s.serial);
                for id in &ids {
                    let Some(pt) = s.get(*id) else { continue };
                    let ObjData::Portal(d) = &pt.data else { continue };
                    let mut built = portal::update(d.clone(), &pt.name, None, 0, &mut Vec::new());
                    portal::invert(&mut built);
                    let mut x = (**pt).clone();
                    x.data = ObjData::Portal(built.data);
                    b.replace.push(x);
                }
                let n = b.replace.len();
                Ok((b, json!({ "inverted": n })))
            })
            .await
        }
        ZoneAction::Mode => {
            let st = set_state(&svc, o, |st| {
                let m = &mut st.sector;
                if let Some(c) = p.click {
                    m.create_single = c == ZoneClick::NewFromMesh;
                    m.create_multiple = c == ZoneClick::NewFromObject;
                }
                if let Some(v) = &p.meshes {
                    m.mesh_add = !v.eq_ignore_ascii_case("remove");
                }
                if let Some(v) = p.box_pick {
                    m.box_pick = v;
                }
                if m.create_single || m.create_multiple {
                    m.box_pick = false;
                }
            })
            .await?;
            let m = st.sector;
            let click = if m.create_single {
                "new_from_mesh"
            } else if m.create_multiple {
                "new_from_object"
            } else {
                "edit"
            };
            Ok(
                json!({ "click": click, "meshes": if m.mesh_add { "add" } else { "remove" }, "box_pick": m.box_pick }),
            )
        }
    }
}

/// The static meshes of `ids` leave their cells and join `cell`; a cell left empty goes with its
/// portals.
fn assign(s: &SceneState, lib: &Library, cell: ObjId, ids: &[ObjId]) -> Result<Outcome, CmdError> {
    let target = s.get(cell).ok_or_else(|| CmdError::not_found("the cell is gone"))?;
    let meshes = meshes_of(s, lib, ids);
    let wanted: FxHashSet<(String, String)> =
        meshes.iter().map(|(a, b)| (a.to_lowercase(), b.to_lowercase())).collect();
    let mut b = Batch::new("Assign To Cell", s.serial);
    let mut moved = 0;
    for c in s.of_class(ObjClass::Sector) {
        if c.id == cell {
            continue;
        }
        let ObjData::Sector(d) = &c.data else { continue };
        let left: Vec<(SmolStr, SmolStr)> = d
            .items
            .iter()
            .filter(|(a, m)| !wanted.contains(&(a.to_lowercase(), m.to_lowercase())))
            .cloned()
            .collect();
        if left.len() == d.items.len() {
            continue;
        }
        if left.is_empty() {
            b.remove.push(c.id);
            b.remove.extend(sector::sector_portals(s, &c.name));
        } else {
            let mut x = (**c).clone();
            if let ObjData::Sector(sd) = &mut x.data {
                sd.items = left;
            }
            b.replace.push(x);
        }
    }
    let mut t = (**target).clone();
    if let ObjData::Sector(d) = &mut t.data {
        for m in meshes {
            if !d.items.iter().any(|i| sector::is_item(i, &m.0, &m.1)) {
                d.items.push(m);
                moved += 1;
            }
        }
    }
    b.replace.push(t);
    Ok((b, json!({ "sector": target.name.as_str(), "moved": moved })))
}

/// `scene.zones action=compute`: the default cell and every portal again (or between the named
/// cells), into the scene.
fn compute(
    s: &SceneState,
    lib: &Library,
    ctl: &super::Ctl,
    only: Option<&[String]>,
) -> Result<Outcome, CmdError> {
    if s.count(ObjClass::Sector) == 0 && only.is_none() {
        return Ok((
            Batch::new("Compute Cells", s.serial),
            json!({ "created": 0, "note": "The level has no cells: it is one cell, and needs no portal." }),
        ));
    }
    let mut b = Batch::new("Compute Cells", s.serial);
    ctl.stage("Filling the default cell");
    let unused = sector::unused_meshes(s, lib);
    let mut work = s.clone();
    let mut default_meshes = 0;
    if !unused.is_empty() && only.is_none() {
        let items: Vec<(SmolStr, SmolStr)> = unused
            .iter()
            .filter_map(|(o, lo, m)| lo.object.meshes.get(*m).map(|mesh| (o.name.clone(), mesh.name.clone())))
            .collect();
        default_meshes = items.len();
        match s.find(ObjClass::Sector, DEFAULT_SECTOR) {
            Some(existing) => {
                let mut x = (**existing).clone();
                if let ObjData::Sector(d) = &mut x.data {
                    d.items.extend(items);
                }
                work.replace(x.clone());
                b.replace.push(x);
            }
            None => {
                let mut x = sector::new_sector(items);
                if let ObjData::Sector(d) = &mut x.data {
                    d.color = [1.0, 0.0, 0.0, 0.0];
                    d.default = true;
                }
                let mut named = x.clone();
                named.name = SmolStr::new(DEFAULT_SECTOR);
                work.insert(named);
                b.add.push((x, NameRule::Fixed(DEFAULT_SECTOR.to_owned())));
            }
        }
    }
    let cells: Vec<ObjId> = match only {
        Some(names) => {
            let ids = named_or_selected(&work, &Some(names.to_vec()), ObjClass::Sector)?;
            if ids.len() < 2 {
                return Err(CmdError::invalid("Name at least two cells to compute the portals between."));
            }
            ids
        }
        None => work.of_class(ObjClass::Sector).map(|o| o.id).collect(),
    };
    let names: Vec<String> =
        cells.iter().filter_map(|id| work.get(*id)).map(|o| o.name.to_lowercase()).collect();
    let between = |p: &crate::objects::PortalData| {
        names.contains(&p.front.to_lowercase()) && names.contains(&p.back.to_lowercase())
    };
    let old: Vec<ObjId> = work
        .of_class(ObjClass::Portal)
        .filter(|p| matches!(&p.data, ObjData::Portal(d) if between(d)))
        .map(|p| p.id)
        .collect();
    b.remove.extend(old.iter().copied());
    let stop = || ctl.stopped();
    let stage = |t: &str| ctl.stage(t);
    let mut n = 0u32;
    let mut next_name = || loop {
        let name = format!("portal_{n:02}");
        n += 1;
        if !work.name_used(&name) {
            return name;
        }
    };
    let c =
        portal::compute(&work, lib, &cells, &mut next_name, &stop, &stage).map_err(|_| super::stopped())?;
    let created = c.portals.len();
    for pd in c.portals {
        b.add.push((
            Object::new(ObjClass::Portal, "", ObjData::Portal(pd)),
            NameRule::Gen(Some("portal".into())),
        ));
    }
    Ok((
        b,
        json!({ "created": created, "removed": old.len(), "default_meshes": default_meshes, "messages": c.log }),
    ))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::objects::SceneObjectData;
    use std::path::PathBuf;

    /// The editor's own cube of the repository's library.
    fn library() -> Library {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..").join("rawdata").join("objects");
        let lib = Library::new(vec![root]);
        lib.load("editor/pcube").unwrap();
        lib
    }

    fn cube(s: &mut SceneState, name: &str, x: f32) -> ObjId {
        let mut o = Object::new(
            ObjClass::SceneObject,
            name,
            ObjData::SceneObject(SceneObjectData::library("editor/pcube")),
        );
        o.xform.position.x = x;
        s.insert(o)
    }

    #[test]
    fn a_level_without_cells_is_one_cell() {
        let lib = library();
        let mut s = SceneState::default();
        cube(&mut s, "a", 0.0);
        let c = complete(&s, &lib, &|| false).ok().unwrap();
        assert_eq!((c.default_meshes, c.portals), (0, 0));
        assert_eq!(c.state.count(ObjClass::Sector), 0);
    }

    #[test]
    fn the_meshes_no_cell_holds_go_into_the_default_cell() {
        let lib = library();
        let mut s = SceneState::default();
        cube(&mut s, "a", 0.0);
        cube(&mut s, "b", 10.0);
        let meshes = lib.cached("editor/pcube").unwrap().object.meshes.len();
        let items: Vec<(SmolStr, SmolStr)> = lib
            .cached("editor/pcube")
            .unwrap()
            .object
            .meshes
            .iter()
            .map(|m| (SmolStr::new("a"), m.name.clone()))
            .collect();
        s.insert(Object::new(
            ObjClass::Sector,
            "room",
            ObjData::Sector(crate::objects::SectorData { items, ..Default::default() }),
        ));
        // the author's cell holds a, so b has no cell: the build validation would refuse it
        let (valid, _) = sector::validate(&s, &lib, None);
        assert!(!valid);
        let c = complete(&s, &lib, &|| false).ok().unwrap();
        assert_eq!(c.default_meshes, meshes);
        let d = c.state.find(ObjClass::Sector, DEFAULT_SECTOR).unwrap();
        let ObjData::Sector(sd) = &d.data else { panic!() };
        assert!(sd.default && sd.items.iter().all(|(o, _)| o == "b"));
        assert!(sector::unused_meshes(&c.state, &lib).is_empty());
        // the author's scene is left as it was
        assert!(s.find(ObjClass::Sector, DEFAULT_SECTOR).is_none());
        assert!(
            c.notes
                .iter()
                .any(|(f, cells)| f.text.contains("default cell") && cells == &[DEFAULT_SECTOR.to_owned()])
        );
    }
}
