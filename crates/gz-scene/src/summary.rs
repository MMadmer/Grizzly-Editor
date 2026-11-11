//! Scene > Summary Info (`SceneSummaryInfo.cpp`): what the scene's tools report of themselves
//! (`GetSummaryInfo`), the textures and library objects it names with what their files say, as
//! a property sheet (`SSceneSummary::FillProp`) and as the text file `ExportSummaryInfo` writes.
//!
//! [`collect`] is pure over a snapshot and the library; [`Summary::prepare`] takes what the files
//! say (texture descriptions, `.seq` lengths) as a lookup, read by the caller on the IO pool.

use std::collections::{BTreeMap, BTreeSet};

use gz_fmt_tex::{BumpMode, TextureFlags, TextureParams};

use crate::class::ObjClass;
use crate::library::Library;
use crate::math::{Aabb, Vec3};
use crate::model::{ObjData, SceneState};
use crate::objects::{self, LT_POINT, LT_SPOT, light_flags};

/// `ESummaryTextureType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum TexKind {
    Base = 0,
    Implicit,
    Detail,
    Bump,
    DetailObject,
    Glow,
    Lod,
}

impl TexKind {
    const ALL: [TexKind; 7] = [
        TexKind::Base,
        TexKind::Implicit,
        TexKind::Detail,
        TexKind::Bump,
        TexKind::DetailObject,
        TexKind::Glow,
        TexKind::Lod,
    ];

    /// `summary_texture_type_tokens`: Detail and Bump have no name there.
    fn token(self) -> &'static str {
        match self {
            TexKind::Base => "Base",
            TexKind::Implicit => "Implicit",
            TexKind::DetailObject => "DO",
            TexKind::Glow => "Glow",
            TexKind::Lod => "LOD",
            TexKind::Detail | TexKind::Bump => "",
        }
    }
}

/// `STextureInfo`.
#[derive(Clone, Debug)]
pub struct TextureInfo {
    pub kind: TexKind,
    /// `STextureParams` of its description; `None` before `prepare` or when there is none.
    pub params: Option<TextureParams>,
    /// The frames of its `.seq`, 1 without one.
    pub frames: u32,
    /// Per object: references and the area of one.
    pub objects: BTreeMap<String, (u32, f32)>,
    pub effective_area: f32,
    pub pixel_area: f32,
}

/// `SObjectInfo`.
#[derive(Clone, Debug, Default)]
pub struct ObjectInfo {
    pub refs: u32,
    /// `References`, `Geometry\Faces` ... under the object's name.
    pub info: Vec<(String, String)>,
}

/// `SSceneSummary`.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    pub level_name: String,
    pub textures: BTreeMap<String, TextureInfo>,
    pub objects: BTreeMap<String, ObjectInfo>,
    pub lod_objects: BTreeSet<String>,
    pub mu_objects: BTreeSet<String>,
    pub waves: BTreeSet<String>,
    pub pe_static: BTreeSet<String>,
    pub face_cnt: i32,
    pub vert_cnt: i32,
    pub hom_face_cnt: i32,
    pub hom_vert_cnt: i32,
    pub snd_occ_face_cnt: i32,
    pub snd_occ_vert_cnt: i32,
    pub object_mu_ref_cnt: i32,
    pub object_lod_ref_cnt: i32,
    pub light_point_cnt: i32,
    pub light_spot_cnt: i32,
    pub light_dynamic_cnt: i32,
    pub light_static_cnt: i32,
    pub light_breakable_cnt: i32,
    pub light_procedural_cnt: i32,
    pub glow_cnt: i32,
    pub sector_cnt: i32,
    pub portal_cnt: i32,
    pub sound_source_cnt: i32,
    pub pe_static_cnt: i32,
    pub bbox: Option<Aabb>,
}

/// `ChangeFileExt(name, "")`, lower case.
fn tex_key(name: &str) -> String {
    let cut = name.rfind('.').filter(|i| !name[*i..].contains(['\\', '/'])).unwrap_or(name.len());
    name[..cut].to_lowercase()
}

/// `printf("%3.2f")`, with MSVC's spelling of what is not a number.
fn f2(v: f32) -> String {
    if v.is_nan() {
        "-nan(ind)".to_owned()
    } else if v.is_infinite() {
        if v > 0.0 { "inf".to_owned() } else { "-inf".to_owned() }
    } else {
        format!("{:.2}", f64::from(v))
    }
}

fn tri_area(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    (b - a).cross(c - a).length() / 2.0
}

impl Summary {
    /// `AppendTexture`.
    fn append_texture(&mut self, name: &str, kind: TexKind, area: f32, pixel_area: f32, object: &str) {
        let t = self.textures.entry(name.to_owned()).or_insert_with(|| TextureInfo {
            kind,
            params: None,
            frames: 1,
            objects: BTreeMap::new(),
            effective_area: 0.0,
            pixel_area: 0.0,
        });
        t.objects.entry(object.to_owned()).or_insert((0, area)).0 += 1;
        t.effective_area += area;
        t.pixel_area += pixel_area;
    }

    /// `AppendObject`.
    fn append_object(&mut self, name: &str) {
        self.objects.entry(name.to_owned()).or_default().refs += 1;
    }

    fn modify_box(&mut self, b: Aabb) {
        match &mut self.bbox {
            Some(a) => {
                a.modify(b.min);
                a.modify(b.max);
            }
            None => self.bbox = Some(b),
        }
    }

    /// `SSceneSummary::Prepare`: each texture's description (`tex(name)` gives its parameters
    /// and the frames of its `.seq`; an implicitly lit one moves to Implicit) and each library
    /// object's rows.
    pub fn prepare(
        &mut self,
        lib: &Library,
        tex: &dyn Fn(&str) -> Option<(TextureParams, u32)>,
    ) -> Vec<String> {
        let mut log = Vec::new();
        for (name, t) in &mut self.textures {
            if name.is_empty() {
                log.push("!Empty texture name found.".to_owned());
                continue;
            }
            match tex(name) {
                Some((p, frames)) => {
                    if p.flags.contains(TextureFlags::IMPLICIT_LIGHTED) {
                        t.kind = TexKind::Implicit;
                    }
                    t.params = Some(p);
                    t.frames = frames;
                }
                None => log.push(format!("!Can't get info from texture: '{name}'")),
            }
        }
        for (name, o) in &mut self.objects {
            if name.is_empty() {
                log.push("!Empty object name found.".to_owned());
                continue;
            }
            let Some(lo) = lib.cached(name) else { continue };
            let e = &lo.object;
            o.info.push((format!("{name}\\References"), o.refs.to_string()));
            o.info.push((format!("{name}\\Geometry\\Faces"), face_count(e).to_string()));
            o.info.push((format!("{name}\\Geometry\\Vertices"), vertex_count(e).to_string()));
            for (si, s) in e.surfaces.iter().enumerate() {
                let pr = format!("{name}\\Materials\\{}", s.name);
                o.info.push((format!("{pr}\\Texture"), s.texture.to_string()));
                o.info.push((format!("{pr}\\Faces"), surf_face_count(e, si).to_string()));
            }
        }
        log
    }

    /// `SSceneSummary::FillProp`: the sheet's rows, key and value, in its order.
    #[must_use]
    pub fn rows(&self) -> Vec<(String, String)> {
        let mut r: Vec<(String, String)> = Vec::new();
        let mut add = |k: &str, v: String| r.push((k.to_owned(), v));
        let b = self.bbox.unwrap_or(Aabb { min: Vec3::splat(f32::MAX), max: Vec3::splat(-f32::MAX) });
        add("Common\\Level Name", self.level_name.clone());
        add("Geometry\\Bounding\\Min", format!("{{{}, {}, {}}}", f2(b.min.x), f2(b.min.y), f2(b.min.z)));
        add("Geometry\\Bounding\\Max", format!("{{{}, {}, {}}}", f2(b.max.x), f2(b.max.y), f2(b.max.z)));
        add("Geometry\\Mesh\\Total Faces", self.face_cnt.to_string());
        add("Geometry\\Mesh\\Total Vertices", self.vert_cnt.to_string());
        add("Geometry\\MU\\Objects", self.mu_objects.len().to_string());
        add("Geometry\\MU\\References", self.object_mu_ref_cnt.to_string());
        add("Geometry\\LOD\\Objects", self.lod_objects.len().to_string());
        add("Geometry\\LOD\\References", self.object_lod_ref_cnt.to_string());
        add("Visibility\\HOM\\Faces", self.hom_face_cnt.to_string());
        add("Visibility\\HOM\\Vertices", self.hom_vert_cnt.to_string());
        add("Visibility\\Sectors", self.sector_cnt.to_string());
        add("Visibility\\Portals", self.portal_cnt.to_string());
        add("Lights\\Count", (self.light_point_cnt + self.light_spot_cnt).to_string());
        add("Lights\\By Type\\Point", self.light_point_cnt.to_string());
        add("Lights\\By Type\\Spot", self.light_spot_cnt.to_string());
        add("Lights\\By Usage\\Dynamic", self.light_dynamic_cnt.to_string());
        add("Lights\\By Usage\\Static", self.light_static_cnt.to_string());
        add("Lights\\By Usage\\Breakable", self.light_breakable_cnt.to_string());
        add("Lights\\By Usage\\Procedural", self.light_procedural_cnt.to_string());
        add("Glows\\Count", self.glow_cnt.to_string());
        for o in self.objects.values() {
            for (k, v) in &o.info {
                add(&format!("Objects\\{k}"), v.clone());
            }
        }
        let mut tex_rows: Vec<(String, String)> = Vec::new();
        let mut total_mem = 0u32;
        for kind in TexKind::ALL {
            let nm = kind.token();
            if nm.is_empty() {
                continue;
            }
            let pref = format!("Textures\\{nm}");
            let mut cur_mem = 0u32;
            let mut cur_area = 0.0f32;
            let mut rows: Vec<(String, String)> = Vec::new();
            for (name, t) in self.textures.iter().filter(|(_, t)| t.kind == kind) {
                cur_area += t.effective_area;
                if name.is_empty() {
                    continue;
                }
                let p = t.params.clone().unwrap_or_else(blank_params);
                let mem = p.memory_usage(t.frames);
                cur_mem = cur_mem.wrapping_add(mem);
                let tp = format!("{pref}\\{name}");
                rows.push((format!("{tp}\\Texture"), name.clone()));
                rows.push((format!("{tp}\\Format"), p.fmt.label().unwrap_or("").to_owned()));
                rows.push((
                    format!("{tp}\\Size"),
                    format!("{} x {} x {}", p.width, p.height, if p.has_alpha() { "32b" } else { "24b" }),
                ));
                rows.push((format!("{tp}\\Memory Usage"), format!("{} Kb", mem / 1024)));
                rows.push((format!("{tp}\\Effective Area"), format!("{} m^2", f2(t.effective_area))));
                rows.push((format!("{tp}\\Pixel Density"), format!("{} p/m", f2(density(t, &p)))));
                if p.flags.intersects(TextureFlags::DIFFUSE_DETAIL | TextureFlags::BUMP_DETAIL) {
                    if p.detail_name.is_empty() {
                        rows.push((format!("{tp}\\Detail Texture"), "INVALID".to_owned()));
                    } else {
                        rows.push((format!("{tp}\\Detail Texture"), p.detail_name.to_string()));
                        rows.push((format!("{tp}\\Detail Scale"), f2(p.detail_scale)));
                    }
                }
                if p.bump_mode == BumpMode::USE {
                    let v =
                        if p.bump_name.is_empty() { "INVALID".to_owned() } else { p.bump_name.to_string() };
                    rows.push((format!("{tp}\\Bump Texture"), v));
                }
            }
            tex_rows.push((format!("{pref}\\Memory Usage"), format!("{} Kb", cur_mem / 1024)));
            tex_rows.push((format!("{pref}\\Effective Area"), format!("{} m^2", f2(cur_area))));
            tex_rows.extend(rows);
            total_mem = total_mem.wrapping_add(cur_mem);
        }
        add("Textures\\Count", self.textures.len().to_string());
        add("Textures\\Memory Usage", format!("{} Kb", total_mem / 1024));
        r.extend(tex_rows);
        let mut add = |k: &str, v: String| r.push((k.to_owned(), v));
        add("Sounds\\Occluder\\Faces", self.snd_occ_face_cnt.to_string());
        add("Sounds\\Occluder\\Vertices", self.snd_occ_vert_cnt.to_string());
        add("Sounds\\Sources", self.sound_source_cnt.to_string());
        add("Sounds\\Waves\\Count", self.waves.len().to_string());
        for w in &self.waves {
            add(&format!("Sounds\\Waves\\{w}"), "-".to_owned());
        }
        add("Particle System\\Sources", self.pe_static_cnt.to_string());
        add("Particle System\\Refs\\Count", self.pe_static.len().to_string());
        for p in &self.pe_static {
            add(&format!("Particle System\\Refs\\{p}"), "-".to_owned());
        }
        r
    }

    /// `ExportSummaryInfo`: the text, every line ended by `\r\n` (`IWriter::w_string`).
    #[must_use]
    pub fn export(&self) -> String {
        let mut out = String::new();
        let mut line = |s: &str| {
            out.push_str(s);
            out.push_str("\r\n");
        };
        line("[TEXTURES]");
        line(
            "texture name=format,width,height,alpha,mem usage (Kb),area,pixel density,objects (name[count*area]),detail name,detail scale,bump name",
        );
        let mut total = 0u32;
        for kind in TexKind::ALL {
            let pref = format!("[{}]", kind.token());
            line(&pref);
            let mut cur_mem = 0u32;
            let mut cur_area = 0.0f32;
            for (name, t) in self.textures.iter().filter(|(_, t)| t.kind == kind) {
                cur_area += t.effective_area;
                let p = t.params.clone().unwrap_or_else(blank_params);
                let mem = p.memory_usage(t.frames);
                cur_mem = cur_mem.wrapping_add(mem);
                let mut objs = String::new();
                for (o, (n, area)) in &t.objects {
                    if !objs.is_empty() {
                        objs.push_str("; ");
                    }
                    objs.push_str(&format!("{o}[{n}*{}]", f2(*area)));
                }
                let mut s = format!(
                    "{name}={},{},{},{},{},{},{},{objs}",
                    p.fmt.label().unwrap_or(""),
                    p.width,
                    p.height,
                    if p.has_alpha() { "present" } else { "absent" },
                    mem / 1024,
                    f2(t.effective_area),
                    f2(density(t, &p)),
                );
                // the mask grows with what the texture has; the arguments stay in their order
                let detail = p.flags.intersects(TextureFlags::DIFFUSE_DETAIL | TextureFlags::BUMP_DETAIL)
                    && !p.detail_name.is_empty();
                let bump = p.bump_mode == BumpMode::USE && !p.bump_name.is_empty();
                // oracle: without the detail fields the bump mask's %s reads the detail name
                match (detail, bump) {
                    (true, true) => {
                        s.push_str(&format!(",{},{},{}", p.detail_name, f2(p.detail_scale), p.bump_name))
                    }
                    (true, false) => s.push_str(&format!(",{},{}", p.detail_name, f2(p.detail_scale))),
                    (false, true) => s.push_str(&format!(",{}", p.detail_name)),
                    (false, false) => {}
                }
                line(&s);
            }
            total = total.wrapping_add(cur_mem);
            line(&format!("{pref} mem usage - {cur_mem} Kb"));
            line(&format!("{pref} effective area - {} m^2", f2(cur_area)));
        }
        line(&format!("Total mem usage - {total} Kb"));
        line("");
        line("[OBJECTS]");
        for o in self.objects.values() {
            for (k, v) in &o.info {
                line(&format!("{k}={v}"));
            }
        }
        out
    }
}

/// `ZeroMemory(&info)`: what a texture without a description reports.
fn blank_params() -> TextureParams {
    TextureParams {
        fmt: gz_fmt_tex::TextureFormat::DXT1,
        flags: TextureFlags::empty(),
        width: 0,
        height: 0,
        bump_mode: BumpMode::RESERVED,
        detail_scale: 0.0,
        ..TextureParams::default()
    }
}

/// `_sqrt((pixel_area*width*height)/effective_area)`.
fn density(t: &TextureInfo, p: &TextureParams) -> f32 {
    ((t.pixel_area * p.width as f32 * p.height as f32) / t.effective_area).sqrt()
}

/// `CEditableObject::GetFaceCount()`: two-sided surfaces twice, `materials\occ` left out.
fn face_count(e: &gz_fmt_object::Object) -> i64 {
    let mut n = 0i64;
    for m in &e.meshes {
        for sf in &m.surf_faces {
            let Some(s) = e.surfaces.get(sf.surface as usize) else { continue };
            if s.game_mtl == "materials\\occ" {
                continue;
            }
            n += sf.faces.len() as i64 * if s.two_sided() { 2 } else { 1 };
        }
    }
    n
}

fn vertex_count(e: &gz_fmt_object::Object) -> i64 {
    e.meshes.iter().map(|m| m.points.len() as i64).sum()
}

/// `GetSurfFaceCount(name)`: its faces, twice when two-sided.
fn surf_face_count(e: &gz_fmt_object::Object, surface: usize) -> i64 {
    let twice = e.surfaces.get(surface).is_some_and(|s| s.two_sided());
    e.meshes
        .iter()
        .flat_map(|m| m.surf_faces.iter())
        .filter(|sf| sf.surface as usize == surface)
        .map(|sf| sf.faces.len() as i64 * if twice { 2 } else { 1 })
        .sum()
}

/// `CalculateSurfaceArea` / `CalculateSurfacePixelArea` of one surface over every mesh.
fn surface_areas(e: &gz_fmt_object::Object, surface: usize) -> (f32, f32) {
    let twice = e.surfaces.get(surface).is_some_and(|s| s.two_sided());
    let (mut area, mut pixel) = (0.0f32, 0.0f32);
    for m in &e.meshes {
        let (mut a, mut p) = (0.0f32, 0.0f32);
        for sf in m.surf_faces.iter().filter(|sf| sf.surface as usize == surface) {
            for &fi in sf.faces.iter() {
                let Some(f) = m.faces.get(fi as usize) else { continue };
                let pt = |k: usize| m.points.get(f.v[k].point as usize).copied().unwrap_or(Vec3::ZERO);
                a += tri_area(pt(0), pt(1), pt(2));
                if let Some(uv) = m.face_uv(fi as usize) {
                    let t = |k: usize| Vec3::new(uv[k].x, uv[k].y, 0.0);
                    p += tri_area(t(0), t(1), t(2));
                }
            }
        }
        if twice {
            a *= 2.0;
            p *= 2.0;
        }
        area += a;
        pixel += p;
    }
    (area, pixel)
}

/// `EScene::CollectSummaryInfo`: every tool's `GetSummaryInfo`, in the tools' order.
#[must_use]
pub fn collect(state: &SceneState, lib: &Library) -> Summary {
    use gz_fmt_object::ObjectFlags as F;
    let mut s = Summary { level_name: state.meta.level.level_path.to_string(), ..Summary::default() };
    let local = |r: &str| lib.cached(r).map(|o| o.bounds);
    let mut classes = ObjClass::ALL.to_vec();
    classes.sort_by_key(|c| c.id());
    for class in classes {
        if class == ObjClass::DetailObjects {
            // `EDetailManager::GetSummaryInfo`: the first surface's texture of each object
            if let Some(d) = &state.bulk.details {
                for r in &d.model.indices.objects {
                    let Some(lo) = lib.cached(&r.reference) else { continue };
                    if let Some(surf) = lo.object.surfaces.first() {
                        s.append_texture(&surf.texture, TexKind::DetailObject, 0.0, 0.0, "$DETAILS$");
                    }
                }
            }
            continue;
        }
        if !class.is_object_tool() {
            continue;
        }
        for o in state.of_class(class) {
            if let Some(b) = objects::world_box(state, o, &local) {
                s.modify_box(b);
            }
            match &o.data {
                ObjData::Glow(g) => {
                    if !g.texture.is_empty() {
                        s.append_texture(&tex_key(&g.texture), TexKind::Glow, 0.0, 0.0, "$GLOW$");
                    }
                    s.glow_cnt += 1;
                }
                ObjData::Light(l) => {
                    match l.kind {
                        LT_POINT => s.light_point_cnt += 1,
                        LT_SPOT => s.light_spot_cnt += 1,
                        _ => {}
                    }
                    if l.kind == LT_POINT || l.kind == LT_SPOT {
                        if l.flags & light_flags::AFFECT_STATIC != 0 {
                            s.light_static_cnt += 1;
                        }
                        if l.flags & light_flags::AFFECT_DYNAMIC != 0 {
                            s.light_dynamic_cnt += 1;
                        }
                        if l.flags & light_flags::PROCEDURAL != 0 {
                            s.light_procedural_cnt += 1;
                        }
                        if l.flags & light_flags::BREAKING != 0 {
                            s.light_breakable_cnt += 1;
                        }
                    }
                }
                ObjData::Portal(_) => s.portal_cnt += 1,
                ObjData::Sector(_) => s.sector_cnt += 1,
                ObjData::SoundSrc(snd) => {
                    if !snd.wav.is_empty() {
                        s.waves.insert(snd.wav.to_string());
                    }
                    s.sound_source_cnt += 1;
                }
                ObjData::SceneObject(d) => {
                    if d.visual_mode() {
                        // a raw visual: the model itself, no editable surfaces
                        if !d.reference.is_empty() {
                            s.append_object(&d.reference);
                        }
                        continue;
                    }
                    let Some(lo) = lib.cached(&d.reference) else { continue };
                    let e = &lo.object;
                    let stat = !e.flags.intersects(F::SOUND_OCCLUDER | F::DYNAMIC | F::HOM);
                    if stat {
                        for si in 0..e.surfaces.len() {
                            let (area, pixel) = surface_areas(e, si);
                            let name = tex_key(&e.surfaces[si].texture);
                            s.append_texture(&name, TexKind::Base, area, pixel, &lo.name);
                        }
                        if e.flags.contains(F::USING_LOD) {
                            s.append_texture(&lod_texture_name(&lo.name), TexKind::Lod, 0.0, 0.0, "$LOD$");
                            s.lod_objects.insert(lo.name.to_string());
                            s.object_lod_ref_cnt += 1;
                        }
                        if e.flags.contains(F::MULTIPLE_USAGE) {
                            s.mu_objects.insert(lo.name.to_string());
                            s.object_mu_ref_cnt += 1;
                        }
                        s.face_cnt += face_count(e) as i32;
                        s.vert_cnt += vertex_count(e) as i32;
                    }
                    if e.flags.contains(F::HOM) {
                        s.hom_face_cnt += face_count(e) as i32;
                        s.hom_vert_cnt += vertex_count(e) as i32;
                    }
                    if e.flags.contains(F::SOUND_OCCLUDER) {
                        s.snd_occ_face_cnt += face_count(e) as i32;
                        s.snd_occ_vert_cnt += vertex_count(e) as i32;
                    }
                    s.append_object(&lo.name);
                }
                _ => {}
            }
        }
    }
    s
}

/// `GetLODTextureName`.
fn lod_texture_name(lib_name: &str) -> String {
    let name = format!("lod_{}", lib_name.replace('\\', "_"));
    match name.split_once('_') {
        Some((folder, _)) => format!("{folder}\\{name}"),
        None => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texture_names_lose_their_extension_and_case() {
        assert_eq!(tex_key("Ston\\Wall.DDS"), "ston\\wall");
        assert_eq!(tex_key("a.b\\c"), "a.b\\c");
    }

    #[test]
    fn an_empty_summary_exports_its_sections() {
        let text = Summary::default().export();
        assert!(text.starts_with("[TEXTURES]\r\n"));
        assert!(text.contains("[Base] mem usage - 0 Kb\r\n"));
        assert!(text.contains("[] effective area - 0.00 m^2\r\n"));
        assert!(text.ends_with("[OBJECTS]\r\n"));
    }
}
