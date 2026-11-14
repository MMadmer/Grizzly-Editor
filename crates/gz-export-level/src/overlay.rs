//! The level's overlay: the collision triangles (`overlay.xcform`), the cuts (the subtractive
//! delta) and the world-space visuals with their registry (`overlay_visuals.ltx`).

use std::sync::Arc;

use gz_base::{Finding, Findings};
use gz_fmt_level::{Aabb, VisualsWriter, XcForm, XcTri, append_cuts, unique_visual_name};
use gz_fmt_object::Object as EditObject;
use gz_fmt_ogf::{ExportOptions, export_static};
use gz_scene::library::LibraryObject;
use gz_scene::{ObjClass, ObjData, Object, SceneState};
use rayon::prelude::*;

use crate::ExportError;

/// A library object by a scene object's reference; `None` when it does not load.
pub type LibraryLookup<'a> = &'a (dyn Fn(&str) -> Option<Arc<LibraryObject>> + Sync);

fn library_of(o: &Object, lib: LibraryLookup<'_>) -> Option<Arc<LibraryObject>> {
    match &o.data {
        ObjData::SceneObject(d) if !d.visual_mode() && !d.reference.is_empty() => lib(&d.reference),
        _ => None,
    }
}

/// What the collision export made.
#[derive(Clone, Debug, PartialEq)]
pub struct Collision {
    pub form: XcForm,
    pub objects_used: u32,
}

/// The collision overlay from the level name on: the scene objects' meshes in world space, a game
/// material per face from its surface (`materials\default` where the surface names none or a
/// face is in no surface), over the cut list `existing` holds.
pub fn collision(
    scene: &SceneState,
    selected_only: bool,
    sector: i64,
    lib: LibraryLookup<'_>,
    existing: &[u8],
) -> Result<Collision, ExportError> {
    if !(0..=65535).contains(&sector) {
        return Err(ExportError::new("sector out of u16 range"));
    }
    let mut verts: Vec<gz_scene::math::Vec3> = Vec::new();
    let mut tris: Vec<XcTri> = Vec::new();
    let mut mtls: Vec<smol_str::SmolStr> = Vec::new();
    let mut objects_used = 0u32;
    for o in scene.of_class(ObjClass::SceneObject) {
        if selected_only && !scene.is_selected(o.id) {
            continue;
        }
        let Some(lo) = library_of(o, lib) else { continue };
        let xf = o.world();
        let mut used = false;
        for mesh in &lo.object.meshes {
            let (vc, fc) = (mesh.points.len(), mesh.faces.len());
            if vc == 0 || fc == 0 {
                continue;
            }
            let base = u32::try_from(verts.len()).map_err(|_| ExportError::new("too many vertices"))?;
            verts.extend(mesh.points.iter().map(|p| xf.transform_tiny(*p)));
            // per-face material via the surface -> faces map; faces no surface covers (broken
            // surface data) fall back to the default game material
            // the material order follows the surfaces in file order (a map keyed by
            // surface identity would follow the heap's addresses).
            let mut fmtl: Vec<Option<usize>> = vec![None; fc];
            for sf in &mesh.surf_faces {
                let gm = lo
                    .object
                    .surfaces
                    .get(sf.surface as usize)
                    .map(|s| s.game_mtl.as_str())
                    .filter(|g| !g.is_empty());
                let mi = XcForm::material_index(&mut mtls, gm.unwrap_or("materials\\default"));
                for face in sf.faces.iter() {
                    if let Some(slot) = fmtl.get_mut(*face as usize) {
                        *slot = Some(mi);
                    }
                }
            }
            for (fi, face) in mesh.faces.iter().enumerate() {
                let mi = match fmtl[fi] {
                    Some(m) => m,
                    None => XcForm::material_index(&mut mtls, "materials\\default"),
                };
                tris.push(XcTri {
                    v: [base + face.v[0].point, base + face.v[1].point, base + face.v[2].point],
                    material: mi as u16,
                });
            }
            used = true;
        }
        if used {
            objects_used += 1;
        }
    }
    if tris.is_empty() {
        return Err(ExportError::new(if selected_only {
            "no triangles collected - select scene objects first"
        } else {
            "no triangles collected - the scene has no static objects"
        }));
    }
    if mtls.len() > 65535 {
        return Err(ExportError::new("too many materials"));
    }
    // keep whatever cuts the cut export left there - only the geometry is ours
    let (mut form, _) = XcForm::read_back(existing);
    form.sector = sector as u16;
    form.verts = verts.into();
    form.tris = tris.into();
    form.materials = mtls;
    Ok(Collision { form, objects_used })
}

/// What the cut export made.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    pub form: XcForm,
    pub ltx: String,
    /// New volumes of the collision side.
    pub added: usize,
}

/// The cut from the level name on: the objects' world bounds (grown by `grow` metres)
/// added to the cut list and to the `[cut_N]` hide entries - both ACCUMULATE, and a box already
/// present (within tolerance) is skipped, so re-exporting the same selection changes nothing.
pub fn cut(
    scene: &SceneState,
    selected_only: bool,
    overlap: bool,
    grow: f32,
    bounds: &dyn Fn(&Object) -> Option<Aabb>,
    xcform: &[u8],
    ltx: &str,
) -> Result<Cut, ExportError> {
    let mut boxes: Vec<Aabb> = Vec::new();
    for o in scene.of_class(ObjClass::SceneObject) {
        if selected_only && !scene.is_selected(o.id) {
            continue;
        }
        let Some(mut b) = bounds(o) else { continue };
        if grow > 0.0 {
            b.min -= gz_scene::math::Vec3::splat(grow);
            b.max += gz_scene::math::Vec3::splat(grow);
        }
        boxes.push(b);
    }
    if boxes.is_empty() {
        return Err(ExportError::new(if selected_only {
            "no objects selected - select the volumes to cut with"
        } else {
            "the scene has no objects to take bounds from"
        }));
    }
    let (mut form, _) = XcForm::read_back(xcform);
    let added = form.add_cuts(&boxes);
    let (ltx, _) = append_cuts(ltx, &boxes, overlap);
    Ok(Cut { form, ltx, added })
}

/// One model the visual export wrote.
#[derive(Clone, Debug)]
pub struct VisualFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

/// What the visual export made.
#[derive(Clone, Debug, Default)]
pub struct VisualOverlay {
    pub files: Vec<VisualFile>,
    /// Skeleton or dynamic references: not world-bakeable.
    pub skipped_dynamic: u32,
    /// A missing `.object` source or an export failure.
    pub skipped_failed: u32,
    pub log: Vec<Finding>,
}

/// The visual overlay from the level name on: every static object exported in world space on a
/// private copy (the shared library instance is never transformed), in parallel on the pool the
/// caller runs it in. `build_name` is the machine stamp of a model (`\\computer\user`); [`registry`]
/// names the files that were written. `materials`, when the game's are known, refuses a
/// surface whose material the game lacks.
pub fn visuals(
    scene: &SceneState,
    selected_only: bool,
    lib: LibraryLookup<'_>,
    build_name: &str,
    materials: Option<std::sync::Arc<gz_fmt_xr::GameMaterials>>,
) -> Result<VisualOverlay, ExportError> {
    let mut out = VisualOverlay::default();
    let mut used: Vec<String> = Vec::new();
    let mut jobs: Vec<(String, EditObject, gz_scene::math::Mat4)> = Vec::new();
    for o in scene.of_class(ObjClass::SceneObject) {
        if selected_only && !scene.is_selected(o.id) {
            continue;
        }
        let ObjData::SceneObject(d) = &o.data else { continue };
        if d.visual_mode() {
            continue;
        }
        // no reference loaded: such an object is passed by without counting it
        let Some(lo) = lib(&d.reference) else { continue };
        // TranslateToWorld only bakes vertices - skeletons keep local bones
        if lo.object.is_skeleton() || lo.object.is_dynamic() {
            out.skipped_dynamic += 1;
            continue;
        }
        let name = unique_visual_name(&o.name, &mut used);
        jobs.push((name, lo.object.clone(), o.world()));
    }
    let exported: Vec<(String, Result<Vec<u8>, String>)> = jobs
        .into_par_iter()
        .map(|(name, mut obj, xf)| {
            for m in &mut obj.meshes {
                let pts: Vec<_> = m.points.iter().map(|p| xf.transform_tiny(*p)).collect();
                m.points = pts.into();
                m.recompute_bbox();
            }
            let opts = ExportOptions {
                source_file: name.as_str().into(),
                build_name: build_name.into(),
                materials: materials.clone(),
            };
            let r = export_static(&obj, &opts).map(|e| e.bytes).map_err(|e| e.to_string());
            (name, r)
        })
        .collect();
    for (name, r) in exported {
        match r {
            Ok(bytes) => {
                out.files.push(VisualFile { name, bytes });
            }
            Err(e) => {
                out.log.warn(format!("[XMS] visual overlay: '{name}' was not exported: {e}"));
                out.skipped_failed += 1;
            }
        }
    }
    if out.files.is_empty() {
        return Err(ExportError::new(if out.skipped_dynamic > 0 || out.skipped_failed > 0 {
            "nothing exported - only skeleton/dynamic or unresolvable objects in the selection"
        } else if selected_only {
            "no scene objects selected"
        } else {
            "the scene has no static objects"
        }));
    }
    Ok(out)
}

/// `overlay_visuals.ltx` with a `[v_N]` section for every file of `names` the existing text does
/// not name yet; numbering goes on after the sections it holds.
#[must_use]
pub fn registry(existing: &str, names: &[String]) -> String {
    let mut writer = VisualsWriter::new(existing);
    for n in names {
        writer.add(n);
    }
    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_fmt_object::{Face, FaceVert, Mesh, SurfFaces, Surface};
    use gz_scene::math::Vec3;
    use gz_scene::objects::SceneObjectData;

    fn unit_box() -> EditObject {
        let p = [
            Vec3::new(-0.5, -0.5, -0.5),
            Vec3::new(0.5, -0.5, -0.5),
            Vec3::new(0.5, 0.5, -0.5),
            Vec3::new(-0.5, 0.5, -0.5),
            Vec3::new(-0.5, -0.5, 0.5),
            Vec3::new(0.5, -0.5, 0.5),
            Vec3::new(0.5, 0.5, 0.5),
            Vec3::new(-0.5, 0.5, 0.5),
        ];
        let idx: [[u32; 3]; 12] = [
            [0, 1, 2],
            [0, 2, 3],
            [4, 6, 5],
            [4, 7, 6],
            [0, 4, 5],
            [0, 5, 1],
            [3, 2, 6],
            [3, 6, 7],
            [0, 3, 7],
            [0, 7, 4],
            [1, 5, 6],
            [1, 6, 2],
        ];
        let faces: Vec<Face> =
            idx.iter().map(|t| Face { v: t.map(|i| FaceVert { point: i, vmref: 0 }) }).collect();
        let mut obj = EditObject::default();
        obj.surfaces.push(Surface {
            name: "box".into(),
            game_mtl: "materials\\concrete".into(),
            ..Surface::default()
        });
        // the last two faces in no surface: they take the default material
        let mesh = Mesh {
            points: p.to_vec().into(),
            faces: faces.into(),
            surf_faces: vec![SurfFaces { surface: 0, faces: (0..10u32).collect::<Vec<_>>().into() }],
            ..Mesh::default()
        };
        obj.meshes.push(mesh);
        obj
    }

    #[test]
    fn a_box_becomes_a_collision_overlay() {
        let lib_obj = Arc::new(LibraryObject::from_object("props\\box", "box.object".into(), unit_box()));
        let mut scene = SceneState::default();
        let mut o = Object::new(
            ObjClass::SceneObject,
            "box",
            ObjData::SceneObject(SceneObjectData::library("props\\box")),
        );
        o.xform.position = Vec3::new(10.0, 0.0, 0.0);
        scene.insert(o);
        let lookup = move |_: &str| Some(lib_obj.clone());
        let c = collision(&scene, false, 3, &lookup, &[]).unwrap();
        assert_eq!(c.objects_used, 1);
        assert_eq!(c.form.sector, 3);
        assert_eq!(c.form.verts.len(), 8);
        assert_eq!(c.form.tris.len(), 12);
        assert_eq!(c.form.materials, ["materials\\concrete", "materials\\default"]);
        assert_eq!(c.form.tris[0].material, 0);
        assert_eq!(c.form.tris[11].material, 1);
        assert!((c.form.verts[0] - Vec3::new(9.5, -0.5, -0.5)).length() < 1e-6);

        let bytes = c.form.write();
        // "XCF1", version 2, sector, reserved, two names
        assert_eq!(&bytes[0..4], b"XCF1");
        assert_eq!(u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]), 2);
        assert_eq!(u16::from_le_bytes([bytes[8], bytes[9]]), 3);
        assert_eq!(u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]), 2);
        let names = b"materials\\concrete\0materials\\default\0";
        assert_eq!(&bytes[16..16 + names.len()], names);
        let total = 16 + names.len() + 4 + 8 * 12 + 4 + 12 * 16 + 4;
        assert_eq!(bytes.len(), total);

        // an export over a file keeps its cuts
        let mut with_cut = c.form.clone();
        with_cut.cuts.push(Aabb::new(Vec3::ZERO, Vec3::ONE));
        let again = collision(&scene, false, 0, &lookup, &with_cut.write()).unwrap();
        assert_eq!(again.form.cuts.len(), 1);
        assert!(
            matches!(collision(&scene, true, 0, &lookup, &[]), Err(ExportError(e)) if e == "no triangles collected - select scene objects first")
        );
        assert!(
            matches!(collision(&scene, false, 70000, &lookup, &[]), Err(ExportError(e)) if e == "sector out of u16 range")
        );
    }

    #[test]
    fn cuts_accumulate() {
        let mut scene = SceneState::default();
        scene.insert(Object::new(
            ObjClass::SceneObject,
            "pit",
            ObjData::SceneObject(SceneObjectData::library("props\\box")),
        ));
        let bounds = |_: &Object| Some(Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0)));
        let first = cut(&scene, false, false, 0.0, &bounds, &[], "").unwrap();
        assert_eq!(first.added, 1);
        assert_eq!(
            first.ltx,
            "[cut_0]\r\nhide    = 0.000000,0.000000,0.000000,1.000000,1.000000,1.000000\r\n; overlap = true  ; uncomment to also take visuals that only touch the box\r\n"
        );
        let second = cut(&scene, false, false, 0.0, &bounds, &first.form.write(), &first.ltx).unwrap();
        assert_eq!(second.added, 0);
        assert_eq!(second.ltx, first.ltx.trim_end().to_owned() + "\r\n\r\n");
    }
}
