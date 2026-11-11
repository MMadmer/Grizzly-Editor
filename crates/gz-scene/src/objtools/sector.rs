//! Sectors: their meshes, their volume, the default sector and the checks
//! (`sector.cpp`, `portalutils.cpp`'s `Validate` / `CreateDefaultSector` / `FindSector`,
//! `ESceneSectorControls.cpp`).

use std::sync::Arc;

use glam::Vec4;
use gz_base::SmolStr;
use gz_editor::{CmdError, Value, json};
use gz_fmt_xr::{ShaderXrlc, ShadersXrlc};

use super::geom::{self, box_holds, box_sphere, boxes_meet, is_static_any};
use super::{Batch, NameRule, Outcome, selected_of};
use crate::class::ObjClass;
use crate::library::{Library, LibraryObject};
use crate::math::{Aabb, Vec3};
use crate::model::{ObjData, ObjId, Object, SceneState};
use crate::objects::{PortalData, SectorData, fcolor_from};

/// `DEFAULT_SECTOR_NAME`, as the scene keeps names.
pub const DEFAULT_SECTOR: &str = "sector_default";

/// A sector's item resolved: the scene object, its library object and the mesh.
#[derive(Clone)]
pub struct Item {
    pub object: Arc<Object>,
    pub lib: Arc<LibraryObject>,
    pub mesh: usize,
}

/// `CSectorItem::IsItem`: names compared without case.
#[must_use]
pub fn is_item(item: &(SmolStr, SmolStr), object: &str, mesh: &str) -> bool {
    item.0.eq_ignore_ascii_case(object) && item.1.eq_ignore_ascii_case(mesh)
}

fn data(o: &Object) -> Option<&SectorData> {
    match &o.data {
        ObjData::Sector(s) => Some(s),
        _ => None,
    }
}

/// The items of a sector that resolve (`LoadSectorDef` keeps only those).
#[must_use]
pub fn items(state: &SceneState, lib: &Library, s: &SectorData) -> Vec<Item> {
    s.items
        .iter()
        .filter_map(|(o, m)| {
            let object = state.find(ObjClass::SceneObject, &o.to_lowercase())?.clone();
            let ObjData::SceneObject(d) = &object.data else { return None };
            let lo = lib.cached(&d.reference)?;
            let mesh = geom::mesh_index(&lo, m)?;
            Some(Item { object, lib: lo, mesh })
        })
        .collect()
}

/// `CPortalUtils::FindSector`: the first sector holding the mesh.
#[must_use]
pub fn find_sector(state: &SceneState, object: &str, mesh: &str) -> Option<ObjId> {
    state
        .of_class(ObjClass::Sector)
        .find(|s| data(s).is_some_and(|d| d.items.iter().any(|i| is_item(i, object, mesh))))
        .map(|s| s.id)
}

/// `CSector::UpdateVolume`: the box of the items' meshes and its sphere.
#[derive(Clone, Copy, Debug)]
pub struct Volume {
    pub bounds: Aabb,
    pub center: Vec3,
    pub radius: f32,
}

#[must_use]
pub fn volume(items: &[Item]) -> Volume {
    let mut bounds = Aabb::INVALID;
    for it in items {
        if let Some(b) = geom::mesh_world_box(&it.lib, it.mesh, &it.object.world()) {
            bounds.modify(b.min);
            bounds.modify(b.max);
        }
    }
    let (center, radius) = box_sphere(&bounds);
    Volume { bounds, center, radius }
}

/// `EVisible`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visible {
    None,
    PartialInside,
    PartialOutside,
    Fully,
}

impl Volume {
    /// `CSector::Intersect(center, radius)`.
    #[must_use]
    pub fn intersect_sphere(&self, center: Vec3, radius: f32) -> Visible {
        let dist = self.center.distance(center);
        let r = self.bounds.half_extents();
        let in_sphere = (dist + radius) < self.radius && radius > r.x && radius > r.y && radius > r.z;
        if box_holds(&self.bounds, center) {
            if in_sphere { Visible::Fully } else { Visible::PartialInside }
        } else if dist < radius + self.radius {
            Visible::PartialOutside
        } else {
            Visible::None
        }
    }

    /// `CSector::Intersect(box)`.
    #[must_use]
    pub fn intersect_box(&self, b: &Aabb) -> Visible {
        if boxes_meet(&self.bounds, b) {
            let (c, r) = box_sphere(b);
            self.intersect_sphere(c, r)
        } else {
            Visible::None
        }
    }
}

/// What `AddMesh` checks: a static object, a mesh no sector holds, not this sector's already.
#[must_use]
pub fn can_add(state: &SceneState, lo: &LibraryObject, items: &[(SmolStr, SmolStr)], object: &str, mesh: &str) -> bool {
    is_static_any(lo) && find_sector(state, object, mesh).is_none() && !items.iter().any(|i| is_item(i, object, mesh))
}

/// The portals that name a sector (`RemoveSectorPortal`).
#[must_use]
pub fn sector_portals(state: &SceneState, sector: &str) -> Vec<ObjId> {
    state
        .of_class(ObjClass::Portal)
        .filter(|p| match &p.data {
            ObjData::Portal(d) => d.front.eq_ignore_ascii_case(sector) || d.back.eq_ignore_ascii_case(sector),
            _ => false,
        })
        .map(|p| p.id)
        .collect()
}

/// `PortalUtils.GetSelectedSector`: the one selected sector. `Ok(None)` when none is.
pub fn selected_sector(state: &SceneState) -> Result<Option<ObjId>, CmdError> {
    let sel = selected_of(state, ObjClass::Sector);
    match sel.as_slice() {
        [] => Ok(None),
        [one] => Ok(Some(*one)),
        _ => Err(CmdError::invalid("Please select only one sector.")),
    }
}

/// `CaptureAllUnusedMeshes`: every mesh of a static scene object no sector holds, in scene
/// order, with its face count.
#[must_use]
pub fn unused_meshes(state: &SceneState, lib: &Library) -> Vec<(Arc<Object>, Arc<LibraryObject>, usize)> {
    let mut out = Vec::new();
    for o in state.of_class(ObjClass::SceneObject) {
        let ObjData::SceneObject(d) = &o.data else { continue };
        let Some(lo) = lib.cached(&d.reference) else { continue };
        if !is_static_any(&lo) {
            continue;
        }
        for (i, m) in lo.object.meshes.iter().enumerate() {
            if find_sector(state, &o.name, &m.name).is_none() {
                out.push((o.clone(), lo.clone(), i));
            }
        }
    }
    out
}

/// `Scene->GetBox(box, OBJCLASS_SCENEOBJECT)`: the box of every scene object that has one.
#[must_use]
pub fn scene_objects_box(state: &SceneState, lib: &Library) -> Option<Aabb> {
    let local = |r: &str| lib.cached(r).map(|o| o.bounds);
    let mut b = Aabb::INVALID;
    let mut any = false;
    for o in state.of_class(ObjClass::SceneObject) {
        if let Some(ob) = crate::objects::world_box(state, o, &local) {
            b.modify(ob.min);
            b.modify(ob.max);
            any = true;
        }
    }
    any.then_some(b)
}

fn shader<'a>(lib: &'a ShadersXrlc, name: &str) -> Option<&'a ShaderXrlc> {
    lib.shaders.iter().find(|s| s.name().eq_ignore_ascii_case(name))
}

/// `CSector::Validate(true)`: what is wrong with one sector, in the C++ words.
#[must_use]
pub fn validate_sector(name: &str, items: &[Item], shaders: Option<&ShadersXrlc>) -> Vec<String> {
    let mut out = Vec::new();
    let faces: usize = items.iter().map(|i| geom::face_count(&i.lib, i.mesh)).sum();
    if faces <= 4 {
        out.push(format!("*ERROR: Sector: '{name}' - face count < 4!"));
    }
    let Some(shaders) = shaders else { return out };
    let mut renderable = false;
    for it in items {
        let Some(m) = it.lib.object.meshes.get(it.mesh) else { continue };
        for sf in &m.surf_faces {
            let Some(surf) = it.lib.object.surfaces.get(sf.surface as usize) else { continue };
            match shader(shaders, &surf.shader_xrlc) {
                None => out.push(format!(
                    "*ERROR: Sector: '{name}' - shader '{}' does not exist on material '{}'.",
                    surf.shader_xrlc, surf.name
                )),
                Some(s) => renderable |= s.flags & ShaderXrlc::RENDERING != 0,
            }
        }
    }
    if !renderable {
        out.push(format!("*ERROR: Sector: '{name}' - can't find any renderable face!"));
    }
    out
}

/// `CPortalUtils::Validate(bMsg)`: whether every static mesh has a sector and every sector is
/// sound, and the C++ messages. `shaders` is the compiler shader library (`None`: not readable,
/// the shader checks are left out).
#[must_use]
pub fn validate(state: &SceneState, lib: &Library, shaders: Option<&ShadersXrlc>) -> (bool, Vec<String>) {
    let mut msgs = Vec::new();
    if scene_objects_box(state, lib).is_none() {
        msgs.push("Validation failed! Can't compute bbox.".to_owned());
        return (false, msgs);
    }
    let mut ok = true;
    let unused = unused_meshes(state, lib);
    let faces: usize = unused.iter().map(|(_, lo, m)| geom::face_count(lo, *m)).sum();
    if faces != 0 {
        msgs.push(format!("*ERROR: Scene has '{faces}' non associated face!"));
        for (o, lo, m) in &unused {
            let mesh = lo.object.meshes.get(*m).map(|x| x.name.as_str()).unwrap_or("");
            msgs.push(format!(
                "! - scene object: '{}' [O:'{}', M:'{}']",
                o.name,
                o.ref_name().unwrap_or(""),
                mesh
            ));
        }
        ok = false;
    }
    for s in state.of_class(ObjClass::Sector) {
        let Some(d) = data(s) else { continue };
        let problems = validate_sector(&s.name, &items(state, lib, d), shaders);
        if !problems.is_empty() {
            ok = false;
            msgs.extend(problems);
        }
    }
    (ok, msgs)
}

/// A new sector as `xr_new<CSector>` makes it.
#[must_use]
pub fn new_sector(items: Vec<(SmolStr, SmolStr)>) -> Object {
    Object::new(ObjClass::Sector, "", ObjData::Sector(SectorData { items, ..SectorData::default() }))
}

fn item_of(o: &Object, lo: &LibraryObject, mesh: usize) -> Option<(SmolStr, SmolStr)> {
    Some((o.name.clone(), lo.object.meshes.get(mesh)?.name.clone()))
}

/// `CreateDefaultSector`: a red default sector of every mesh no sector holds, selected.
pub fn create_default(state: &SceneState, lib: &Library) -> Result<Outcome, CmdError> {
    let refused = || CmdError::invalid("The default sector could not be created");
    if state.find(ObjClass::Sector, DEFAULT_SECTOR).is_some() {
        return Err(CmdError::invalid("The default sector is there already"));
    }
    if scene_objects_box(state, lib).is_none() {
        return Err(refused());
    }
    let unused = unused_meshes(state, lib);
    let faces: usize = unused.iter().map(|(_, lo, m)| geom::face_count(lo, *m)).sum();
    if faces == 0 {
        return Err(refused());
    }
    let list: Vec<(SmolStr, SmolStr)> = unused.iter().filter_map(|(o, lo, m)| item_of(o, lo, *m)).collect();
    let n = list.len();
    let mut s = new_sector(list);
    if let ObjData::Sector(d) = &mut s.data {
        d.color = [1.0, 0.0, 0.0, 0.0];
        d.default = true;
    }
    let mut b = Batch::new("Create Default Sector", state.serial);
    b.add.push((s, NameRule::Fixed(DEFAULT_SECTOR.to_owned())));
    b.select_added = true;
    Ok((b, json!({ "sector": DEFAULT_SECTOR, "meshes": n, "faces": faces })))
}

/// `RemoveDefaultSector`, with the portals of it (`CSector::OnDestroy`).
pub fn remove_default(state: &SceneState) -> Result<Outcome, CmdError> {
    let s = state
        .find(ObjClass::Sector, DEFAULT_SECTOR)
        .ok_or_else(|| CmdError::not_found("There is no default sector"))?;
    let mut b = Batch::new("Remove Default Sector", state.serial);
    b.remove.push(s.id);
    let portals = sector_portals(state, &s.name);
    b.remove.extend(portals.iter().copied());
    Ok((b, json!({ "removed": DEFAULT_SECTOR, "portals_removed": portals.len() })))
}

/// `CSector::CaptureInsideVolume`: the meshes of the static scene objects whose boxes are fully
/// or partly inside the sector's volume join it.
pub fn capture_volume(state: &SceneState, lib: &Library, id: ObjId) -> Result<Outcome, CmdError> {
    let sector = state.get(id).ok_or_else(|| CmdError::not_found("sector not found"))?;
    let d = data(sector).ok_or_else(|| CmdError::invalid("the object is not a sector"))?;
    let vol = volume(&items(state, lib, d));
    let local = |r: &str| lib.cached(r).map(|o| o.bounds);
    let mut list = d.items.clone();
    let mut added = 0;
    for o in state.of_class(ObjClass::SceneObject) {
        if !o.flags.visible {
            continue;
        }
        // `Scene->SpherePick`: the object's box sphere meets the sector's
        let Some(ob) = crate::objects::world_box(state, o, &local) else { continue };
        let (c, r) = box_sphere(&ob);
        let rr = r + vol.radius;
        if c.distance_squared(vol.center) >= rr * rr {
            continue;
        }
        let ObjData::SceneObject(sd) = &o.data else { continue };
        let Some(lo) = lib.cached(&sd.reference) else { continue };
        if !is_static_any(&lo) {
            continue;
        }
        let world = o.world();
        for (i, m) in lo.object.meshes.iter().enumerate() {
            let Some(bb) = geom::mesh_world_box(&lo, i, &world) else { continue };
            if matches!(vol.intersect_box(&bb), Visible::Fully | Visible::PartialInside)
                && can_add(state, &lo, &list, &o.name, &m.name)
            {
                list.push((o.name.clone(), m.name.clone()));
                added += 1;
            }
        }
    }
    let mut b = Batch::new("Capture Volume", state.serial);
    if added > 0 {
        let mut s = (**sector).clone();
        if let ObjData::Sector(sd) = &mut s.data {
            sd.items = list;
        }
        b.replace.push(s);
    }
    Ok((b, json!({ "sector": sector.name.as_str(), "added": added })))
}

/// One edit of a sector's meshes (`AddMesh` / `DelMesh`) with what it says.
#[derive(Default)]
struct MeshEdits {
    added: usize,
    removed: usize,
    emptied: bool,
}

/// `AddMesh` / `DelMesh` of a list of meshes on the sector: a gesture's click, its paint or its
/// box. `first_only_per_object` is the box's Remove rule: after a mesh of an object came out, the
/// object's other meshes are not looked at. A sector that loses its last mesh goes, with its
/// portals.
pub fn edit_meshes(
    state: &SceneState,
    lib: &Library,
    id: ObjId,
    picks: &[(ObjId, usize)],
    add: bool,
    first_only_per_object: bool,
    label: &str,
) -> Result<Outcome, CmdError> {
    let sector = state.get(id).ok_or_else(|| CmdError::not_found("sector not found"))?;
    let d = data(sector).ok_or_else(|| CmdError::invalid("the object is not a sector"))?;
    let mut list = d.items.clone();
    let mut e = MeshEdits::default();
    let mut skip: Option<ObjId> = None;
    for (oid, mesh) in picks {
        if e.emptied {
            break;
        }
        if skip == Some(*oid) {
            continue;
        }
        let Some(o) = state.get(*oid) else { continue };
        let ObjData::SceneObject(sd) = &o.data else { continue };
        let Some(lo) = lib.cached(&sd.reference) else { continue };
        let Some(m) = lo.object.meshes.get(*mesh) else { continue };
        if add {
            if can_add(state, &lo, &list, &o.name, &m.name) {
                list.push((o.name.clone(), m.name.clone()));
                e.added += 1;
            }
        } else {
            let before = list.len();
            list.retain(|i| !is_item(i, &o.name, &m.name));
            let hit = list.len() != before;
            if hit {
                e.removed += 1;
            }
            if list.is_empty() {
                e.emptied = true;
            }
            if (hit || e.emptied) && first_only_per_object {
                skip = Some(*oid);
            }
        }
    }
    let mut b = Batch::new(label, state.serial);
    let mut out = json!({ "sector": sector.name.as_str(), "added": e.added, "removed": e.removed });
    if e.emptied {
        b.remove.push(id);
        let portals = sector_portals(state, &sector.name);
        b.remove.extend(portals);
        if let Value::Object(m) = &mut out {
            m.insert("sector_removed".into(), json!(true));
            m.insert("note".into(), json!("Last mesh deleted.\nSector has no meshes and will be removed."));
        }
    } else if e.added > 0 || e.removed > 0 {
        let mut s = (**sector).clone();
        if let ObjData::Sector(sd) = &mut s.data {
            sd.items = list;
        }
        b.replace.push(s);
    }
    Ok((b, out))
}

/// `TUI_ControlSectorAdd::AddSector`: a new sector of one mesh, selected.
pub fn new_from_mesh(state: &SceneState, lib: &Library, oid: ObjId, mesh: usize) -> Result<Outcome, CmdError> {
    let o = state.get(oid).ok_or_else(|| CmdError::not_found("object not found"))?;
    let ObjData::SceneObject(sd) = &o.data else { return Err(CmdError::invalid("the object is not a scene object")) };
    let lo = lib.cached(&sd.reference).ok_or_else(|| CmdError::invalid("the object's library object is not loaded"))?;
    let m = lo.object.meshes.get(mesh).ok_or_else(|| CmdError::not_found("the object has no such mesh"))?;
    if !can_add(state, &lo, &[], &o.name, &m.name) {
        return Err(CmdError::invalid("The mesh cannot make a sector: it is dynamic or in a sector already."));
    }
    let mut b = Batch::new("New Sector From Mesh", state.serial);
    b.add.push((new_sector(vec![(o.name.clone(), m.name.clone())]), NameRule::Gen(None)));
    b.select_added = true;
    Ok((b, json!({ "object": o.name.as_str(), "mesh": m.name.as_str() })))
}

/// `valid_color`: not white, grey, black, red or dark red.
#[must_use]
pub fn valid_color(r: u32, g: u32, b: u32) -> bool {
    !matches!((r, g, b), (255, 255, 255) | (127, 127, 127) | (0, 0, 0) | (255, 0, 0) | (127, 0, 0))
}

/// `TUI_ControlSectorAdd::AddSectors`: a sector of each mesh of the object, each in a colour of
/// its own; the last one stays selected.
pub fn new_from_object(state: &SceneState, lib: &Library, oid: ObjId, seed: i32) -> Result<Outcome, CmdError> {
    let o = state.get(oid).ok_or_else(|| CmdError::not_found("object not found"))?;
    let ObjData::SceneObject(sd) = &o.data else { return Err(CmdError::invalid("the object is not a scene object")) };
    let lo = lib.cached(&sd.reference).ok_or_else(|| CmdError::invalid("the object's library object is not loaded"))?;
    let mut rng = geom::CRandom::new(seed);
    let mut b = Batch::new("New Sectors From Object", state.serial);
    b.select_added = true;
    let mut names = Vec::new();
    for m in &lo.object.meshes {
        if !can_add(state, &lo, &[], &o.name, &m.name) {
            continue;
        }
        let (r, g, bl) = loop {
            let mut c = || (rng.rand_range(0, 3) * 255 / 2) as u32;
            let (r, g, bl) = (c(), c(), c());
            if valid_color(r, g, bl) {
                break (r, g, bl);
            }
        };
        let mut s = new_sector(vec![(o.name.clone(), m.name.clone())]);
        if let ObjData::Sector(d) = &mut s.data {
            d.color = fcolor_from((r << 16) | (g << 8) | bl);
        }
        b.add.push((s, NameRule::Gen(None)));
        names.push(m.name.to_string());
    }
    if b.add.is_empty() {
        return Err(CmdError::invalid("No mesh of the object can make a sector: it is dynamic or in sectors already."));
    }
    Ok((b, json!({ "object": o.name.as_str(), "meshes": names })))
}

/// The meshes of the visible scene objects a box pick's frustum holds, in scene order
/// (`FrustumPick` of the objects, then of their meshes).
#[must_use]
pub fn meshes_in_frustum(state: &SceneState, lib: &Library, planes: &[Vec4]) -> Vec<(ObjId, usize)> {
    let local = |r: &str| lib.cached(r).map(|o| o.bounds);
    let mut out = Vec::new();
    for o in state.of_class(ObjClass::SceneObject) {
        if !o.flags.visible {
            continue;
        }
        let Some(ob) = crate::objects::world_box(state, o, &local) else { continue };
        if !geom::box_in_frustum(planes, &ob) {
            continue;
        }
        let ObjData::SceneObject(sd) = &o.data else { continue };
        let Some(lo) = lib.cached(&sd.reference) else { continue };
        let world = o.world();
        for i in 0..lo.object.meshes.len() {
            if geom::mesh_in_frustum(&lo, i, &world, planes) {
                out.push((o.id, i));
            }
        }
    }
    out
}

/// What `sector_tool` lists of a sector: its name, the default mark, counts (`GetCounts`).
#[must_use]
pub fn sector_json(state: &SceneState, lib: &Library, o: &Object) -> Value {
    let Some(d) = data(o) else { return Value::Null };
    let its = items(state, lib, d);
    let faces: usize = its.iter().map(|i| geom::face_count(&i.lib, i.mesh)).sum();
    let mut objects: Vec<&str> = d.items.iter().map(|(o, _)| o.as_str()).collect();
    objects.sort_unstable();
    objects.dedup();
    json!({
        "name": o.name.as_str(),
        "default": d.default,
        "selected": state.is_selected(o.id),
        "objects": objects.len(),
        "meshes": d.items.len(),
        "faces": faces,
        "portals": sector_portals(state, &o.name).len(),
    })
}

/// Whether a portal is between two named sectors.
#[must_use]
pub fn portal_between(p: &PortalData, a: &str, b: &str) -> bool {
    (p.front.eq_ignore_ascii_case(a) && p.back.eq_ignore_ascii_case(b))
        || (p.front.eq_ignore_ascii_case(b) && p.back.eq_ignore_ascii_case(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sector_colours_leave_out_the_reserved_ones() {
        assert!(!valid_color(255, 0, 0));
        assert!(!valid_color(127, 127, 127));
        assert!(valid_color(0, 127, 255));
    }

    #[test]
    fn a_box_inside_the_volume_is_partly_inside() {
        let v = Volume {
            bounds: Aabb::new(Vec3::splat(-5.0), Vec3::splat(5.0)),
            center: Vec3::ZERO,
            radius: Vec3::splat(5.0).length(),
        };
        let small = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        assert_eq!(v.intersect_box(&small), Visible::PartialInside);
        let far = Aabb::new(Vec3::splat(20.0), Vec3::splat(21.0));
        assert_eq!(v.intersect_box(&far), Visible::None);
    }

    #[test]
    fn the_default_sector_is_refused_without_objects() {
        let s = SceneState::default();
        let lib = Library::new(Vec::new());
        assert!(create_default(&s, &lib).is_err());
        let (ok, msgs) = validate(&s, &lib, None);
        assert!(!ok);
        assert_eq!(msgs, vec!["Validation failed! Can't compute bbox.".to_owned()]);
    }
}
