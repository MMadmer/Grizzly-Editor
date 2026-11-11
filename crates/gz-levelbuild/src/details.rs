//! `EDetailManager::Export`: `build.details` - the detail objects the slots use, their
//! textures merged into `build_details.dds`, the slots with their colours zeroed for the light
//! compiler.

use std::collections::BTreeSet;
use std::sync::Arc;

use glam::{Vec2, Vec3};
use gz_fmt_level::{DETAIL_VERSION, DetailModel, DetailSlot, DetailVertex, LevelDetails, SceneDetails};
use gz_fmt_object::VMapKind;
use gz_fmt_tex::{
    Image, MipFilterId, TextureFlags, TextureFormat, TextureParams, TextureType, make_game_texture,
};
use gz_scene::library::LibraryObject;

use crate::merge::{self, SourceImage};
use crate::{BuildError, Result};

/// The merged texture's name, as the objects name it.
pub const DETAILS_TEX_NAME: &str = "build_details";
const EPS_L: f32 = 0.0001;

/// A library object by reference.
pub type Lookup<'a> = &'a (dyn Fn(&str) -> Option<Arc<LibraryObject>> + Sync);

/// The texture of a detail object: its first surface's.
fn texture_of(lo: &LibraryObject) -> Option<String> {
    lo.object.surfaces.first().map(|s| s.texture.to_string())
}

/// The textures the slots use, sorted, each once: what [`export`] needs read first.
#[must_use]
pub fn used_textures(details: &SceneDetails, lib: Lookup<'_>) -> Vec<String> {
    let objects = &details.indices.objects;
    let mut set = BTreeSet::new();
    for slot in details.slots.iter() {
        for part in 0..4 {
            let id = slot.id(part);
            if id == DetailSlot::ID_EMPTY {
                continue;
            }
            if let Some(t) =
                objects.get(usize::from(id)).and_then(|o| lib(&o.reference)).and_then(|lo| texture_of(&lo))
            {
                set.insert(t);
            }
        }
    }
    set.into_iter().collect()
}

/// `EDetail::Update`: the first mesh as an indexed list, equal corners welded, degenerate and
/// repeated triangles dropped.
fn geometry(lo: &LibraryObject) -> (Vec<DetailVertex>, Vec<u16>) {
    let mut verts: Vec<DetailVertex> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    let Some(m) = lo.object.meshes.first() else { return (verts, indices) };
    let similar = |a: &DetailVertex, b: &DetailVertex| {
        (a.u - b.u).abs() <= EPS_L
            && (a.v - b.v).abs() <= EPS_L
            && (a.pos - b.pos).abs().max_element() <= EPS_L
    };
    for f in m.faces.iter() {
        let mut ind = [0u16; 3];
        for (slot, fv) in ind.iter_mut().zip(f.v.iter()) {
            let p = m.points.get(fv.point as usize).copied().unwrap_or(Vec3::ZERO);
            let uv = m
                .vmrefs
                .get(fv.vmref as usize)
                .and_then(|r| r.first())
                .and_then(|pt| {
                    m.vmaps
                        .get(pt.vmap as usize)
                        .filter(|vm| vm.kind == VMapKind::Uv)
                        .map(|vm| (vm, pt.index as usize * 2))
                })
                .map_or(Vec2::ZERO, |(vm, i)| {
                    Vec2::new(
                        vm.values.get(i).copied().unwrap_or(0.0),
                        vm.values.get(i + 1).copied().unwrap_or(0.0),
                    )
                });
            let v = DetailVertex { pos: p, u: uv.x, v: uv.y };
            *slot = match verts.iter().position(|w| similar(w, &v)) {
                Some(i) => i as u16,
                None => {
                    verts.push(v);
                    (verts.len() - 1) as u16
                }
            };
        }
        if ind[0] == ind[1] || ind[1] == ind[2] || ind[0] == ind[2] {
            continue;
        }
        if indices.as_chunks::<3>().0.contains(&ind) {
            continue;
        }
        indices.extend_from_slice(&ind);
    }
    (verts, indices)
}

/// `build.details` and `build_details.dds`. `textures` gives a used texture's RGBA pixels by
/// name, as [`used_textures`] listed them.
// oracle: the C++ orders the textures by shared-string address before packing them; here they
// are in name order.
/// A texture by name: its RGBA and size.
pub type TextureSource<'a> = &'a dyn Fn(&str) -> Option<(Vec<u8>, u32, u32)>;

pub fn export(
    details: &SceneDetails,
    lib: Lookup<'_>,
    textures: TextureSource<'_>,
) -> Result<(Vec<u8>, Vec<u8>)> {
    let objects = &details.indices.objects;
    let names = used_textures(details, lib);
    let mut used = vec![false; objects.len()];
    for slot in details.slots.iter() {
        for part in 0..4 {
            let id = slot.id(part);
            if id != DetailSlot::ID_EMPTY && usize::from(id) < used.len() {
                used[usize::from(id)] = true;
            }
        }
    }
    let mut remap = vec![u8::MAX; objects.len()];
    let mut next = 0u8;
    for (i, o) in objects.iter().enumerate() {
        let tex = lib(&o.reference).and_then(|lo| texture_of(&lo));
        if used[i] && tex.is_some_and(|t| names.contains(&t)) {
            remap[i] = next;
            next = next.wrapping_add(1);
        }
    }
    let mut images = Vec::with_capacity(names.len());
    for n in &names {
        let Some((rgba, w, h)) = textures(n) else {
            return Err(BuildError::new(format!("Can't load texture '{n}'. Check file existence.")));
        };
        images.push(SourceImage { name: n.clone(), w, h, layers: vec![rgba] });
    }
    let merged = merge::merge_textures(1, images, 256, 1024, 256, 1024)
        .ok_or_else(|| BuildError::new("Export failed."))?;
    let image = Image::new(merged.w, merged.h, merged.layers.first().cloned().unwrap_or_default())
        .map_err(|e| BuildError::new(e.to_string()))?;
    let params = TextureParams {
        fmt: TextureFormat::DXT5,
        ty: TextureType::IMAGE,
        mip_filter: MipFilterId::ADVANCED,
        flags: TextureFlags::DITHER_COLOR | TextureFlags::GENERATE_MIP_MAPS,
        width: merged.w as i32,
        height: merged.h as i32,
        ..TextureParams::default()
    };
    let dds = make_game_texture(&image, &params, None).map_err(|e| BuildError::new(e.to_string()))?.dds;

    let mut models = Vec::new();
    for (i, o) in objects.iter().enumerate() {
        if remap[i] == u8::MAX {
            continue;
        }
        let Some(lo) = lib(&o.reference) else {
            return Err(BuildError::new(format!("Bad object or object not found '{}'.", o.reference)));
        };
        let surf =
            lo.object.surfaces.first().ok_or_else(|| {
                BuildError::new(format!("Bad object or object not found '{}'.", o.reference))
            })?;
        let t = names.iter().position(|n| *n == surf.texture.as_str()).map_or(0, |t| merged.remap[t]);
        let (verts, indices) = geometry(&lo);
        let verts = verts
            .into_iter()
            .map(|v| {
                let uv = merge::remap_uv(
                    Vec2::new(v.u, v.v),
                    merged.offsets[t],
                    merged.scales[t],
                    merged.rotated[t],
                );
                DetailVertex { pos: v.pos, u: uv.x, v: uv.y }
            })
            .collect();
        models.push(DetailModel {
            shader: surf.shader.clone(),
            texture: DETAILS_TEX_NAME.into(),
            flags: o.flags,
            min_scale: o.min_scale,
            max_scale: o.max_scale,
            verts,
            indices,
        });
    }
    let slots: Vec<DetailSlot> = details.slots.iter().map(|s| s.for_export(&remap)).collect();
    let mut header = details.header;
    header.version = DETAIL_VERSION;
    header.object_count = models.len() as u32;
    let file = LevelDetails { header, models, slots: slots.into(), chunk_order: LevelDetails::BUILD_ORDER };
    Ok((file.write(), dds))
}
