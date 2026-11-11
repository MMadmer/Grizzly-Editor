//! A scene snapshot as the renderer's `DrawList`, pure (`EScene::Render` and each class's
//! `Render`): every object's meshes as instances by handle, and the helper shapes each class
//! draws - light and sound ranges, shapes, way points and links, spawn marks, portals, sector
//! bounds, the selection boxes - in the C++ colours and forms. Labels are the renderer's: it
//! tells which are visible and the overlay draws them.

use std::sync::Arc;

use glam::{Mat4, Quat, Vec3};
use gz_fmt_object::Mat4 as XMat;
use gz_render::{
    Aabb, Assets, Color, DrawList, Environment, Helpers, Instance, InstanceFlags, Light, ShapeKind,
    Visibility,
};
use gz_scene::library::Library;
use gz_scene::objects::{
    self, CF_SPHERE, FV_OCCLUSION, LT_POINT, LT_SPOT, PLAYERSTART_HALF_WIDTH, PointKind, ShapeData,
    VIS_RADIUS, WAYPOINT_RADIUS, WAYPOINT_SIZE, light_flags,
};
use gz_scene::pick::ScenePick;
use gz_scene::{ObjClass, ObjData, ObjId, Object, SceneState};

/// `ESceneSpawnTool::flShowSpawnType`, `flPickSpawnType`.
const SPAWN_SHOW_TYPE: u32 = 1 << 31;
const SPAWN_PICK_TYPE: u32 = 1 << 30;
/// `ESceneLightTool::flShowControlName`.
const LIGHT_SHOW_NAME: u32 = 1 << 30;
/// `ESceneGlowTool::flDrawCross`.
const GLOW_DRAW_CROSS: u32 = 1 << 30;

/// `ELight.cpp`: `SEL_COLOR`, `NORM_COLOR`, `NORM_DYN_COLOR`.
const LIGHT_SEL: u32 = 0x00FF_FFFF;
const LIGHT_NORM: u32 = 0x00FF_FF00;
const LIGHT_DYN: u32 = 0x0000_FF00;
/// `ESound_Source.cpp`.
const SOUND_SEL0: u32 = 0x00A0_A0F0;
const SOUND_SEL1: u32 = 0x00FF_FFFF;
const SOUND_NORM: u32 = 0x0000_00FF;
/// `EShape.cpp`, the sound environment's and the fog volume's `SetDrawColor`.
const SHAPE_TRANSP: u32 = 0x3C80_8080;
const SHAPE_EDGE: u32 = 0xFF20_2020;
const ENV_TRANSP: u32 = 0x2050_50FF;
const FOG_OCCLUSION_TRANSP: u32 = 0x2050_A050;
const FOG_GROUP_EDGE: u32 = 0xFFFF_2020;
/// `SpawnPoint.cpp`.
const PLAYERSTART_COLOR: u32 = 0xFF30_C060;
const RP_COLORS: [u32; 6] = [0xff0000, 0x00ff00, 0x0000ff, 0xffff00, 0x00ffff, 0xff00ff];
const ENVMOD_COLOR: u32 = 0x00FF_AE00;
/// `CGroupObject::Render`.
const GROUP_SEL: u32 = 0xFF70_70FF;
/// `glow.cpp`'s romboid, `EParticlesObject`'s cross.
const GLOW_COLOR: u32 = 0x00FF_8507;
const PARTICLES_COLOR: u32 = 0xFFFF_EBAA;
/// A scene object of the snap list (`RenderSnapList` draws it with the selection shader).
const SNAP_TINT: Color = Color::rgb(150, 210, 255);

/// What a frame shows besides the scene's surfaces.
#[derive(Clone, Debug)]
pub struct DrawOptions {
    /// Spawn objects' shapes, marks, icons, labels and selection boxes (`g_SpawnHelpers`).
    pub helpers: bool,
    /// The game-lighting mode's weather and the lights it draws.
    pub game: Option<(Environment, Vec<Light>)>,
}

/// The draw list and, per instance, the object it draws.
#[derive(Clone, Debug, Default)]
pub struct Built {
    pub list: DrawList,
    pub owners: Vec<ObjId>,
}

/// An engine matrix (rows `i`, `j`, `k`, `c`) as the renderer's.
#[must_use]
pub fn gm(m: &XMat) -> Mat4 {
    Mat4::from_cols_array_2d(&m.0)
}

/// A scene box as the renderer's.
#[must_use]
pub fn ga(b: &gz_fmt_level::Aabb) -> Option<Aabb> {
    b.is_valid().then(|| Aabb::new(b.min, b.max))
}

/// A line colour from the engine's `0xAARRGGBB`: lines ignore the alpha the C++ passes.
fn line(argb: u32) -> Color {
    Color::from_argb(argb).with_alpha(255)
}

fn fill(argb: u32) -> Color {
    Color::from_argb(argb)
}

/// `subst_alpha(c, a)`.
fn with_alpha(argb: u32, a: u32) -> Color {
    Color::from_argb((argb & 0x00FF_FFFF) | (a.min(255) << 24))
}

/// `DrawCross(p, szx1, szy1, szz1, szx2, szy2, szz2)`: the arms may differ each way.
#[allow(clippy::too_many_arguments)]
fn cross6(h: &mut Helpers, p: Vec3, x1: f32, y1: f32, z1: f32, x2: f32, y2: f32, z2: f32, c: Color) {
    h.line(Vec3::new(p.x + x2, p.y, p.z), Vec3::new(p.x - x1, p.y, p.z), c, true);
    h.line(Vec3::new(p.x, p.y + y2, p.z), Vec3::new(p.x, p.y - y1, p.z), c, true);
    h.line(Vec3::new(p.x, p.y, p.z + z2), Vec3::new(p.x, p.y, p.z - z1), c, true);
}

/// `DrawLineSphere(p, r, c, bCross)`.
fn line_sphere(h: &mut Helpers, p: Vec3, r: f32, c: Color, cross: bool) {
    h.sphere(p, r, None, Some(c), true);
    if cross {
        h.cross(p, r, false, c, true);
    }
}

/// `DrawSelectionBoxB`: the corners of a box carried by a matrix.
fn selection_box(h: &mut Helpers, local: &Aabb, world: &Mat4, c: Color) {
    if local.is_empty() {
        return;
    }
    h.shape(ShapeKind::SelectionBox, *world * local.unit_transform(), None, Some(c), true);
}

fn romboid(h: &mut Helpers, p: Vec3, r: f32, c: Color) {
    h.shape(
        ShapeKind::Romboid,
        Mat4::from_scale_rotation_translation(Vec3::splat(r), Quat::IDENTITY, p),
        None,
        Some(c),
        true,
    );
}

/// `DrawEntity`: the flag of a spawn point in its own space, and its icon's square.
fn entity_flag(h: &mut Helpers, rp: &Mat4, c: Color) {
    let pts = [
        Vec3::ZERO,
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, 1.0, 0.5),
        Vec3::new(0.0, 0.5, 0.5),
        Vec3::new(0.0, 0.5, 0.0),
    ];
    for w in pts.windows(2) {
        if let [a, b] = w {
            h.line(rp.transform_point3(*a), rp.transform_point3(*b), c, true);
        }
    }
    // the square the icon is drawn on: the XZ plane turned to face +X, between y .5..1, z 0...5
    let plane = *rp
        * Mat4::from_translation(Vec3::new(0.0, 0.75, 0.25))
        * Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_2)
        * Mat4::from_scale(Vec3::splat(0.5));
    h.shape(ShapeKind::Plane, plane, Some(c.with_alpha(140)), None, true);
}

/// A shape's spheres and boxes in the object's space (`CEditShape::Render`).
fn shapes(h: &mut Helpers, data: &ShapeData, world: &Mat4, transp: Color, edge: Color, selected: bool) {
    let a = u32::from(transp.0[3]);
    let fill_c = if selected { transp.with_alpha((a * 2).min(255) as u8) } else { transp };
    for s in &data.shapes {
        if s.kind == CF_SPHERE {
            let b = *world
                * Mat4::from_scale_rotation_translation(Vec3::splat(s.radius()), Quat::IDENTITY, s.center());
            h.shape(ShapeKind::Cross { rot45: false }, b, None, Some(edge), true);
            h.shape(ShapeKind::Sphere, b, Some(fill_c), Some(edge), true);
        } else {
            let b = *world * gm(&s.matrix());
            h.shape(ShapeKind::Box, b, Some(fill_c), Some(edge), true);
        }
    }
    if selected && let Some(local) = ga(&data.local_box()) {
        selection_box(h, &local, world, Color::WHITE);
    }
}

fn world_box(state: &SceneState, lib: &Library, o: &Object) -> Option<Aabb> {
    let local = |r: &str| lib.cached(r).map(|x| x.bounds);
    objects::world_box(state, o, &local).and_then(|b| ga(&b))
}

/// The name a spawn point's label shows (`flShowSpawnType`).
fn spawn_label(o: &Object, d: &objects::SpawnData) -> String {
    if let Some(s) = d.section() {
        return s.to_owned();
    }
    match d.kind {
        PointKind::RPoint => format!("RPoint T:{}", d.rpoint.team),
        PointKind::EnvMod => format!("EnvMod V:{:3.2}, F:{:3.2}", d.env.view_dist, d.env.fog_density),
        PointKind::PlayerStart => "Player Start".to_owned(),
        PointKind::Spawn => o.name.to_string(),
    }
}

/// `RP_COLORS[team % 6]` dimmed by the team's number.
fn rpoint_color(team: u8) -> Color {
    let k = 1.0 / ((f32::from(team) + 1.0) / 6.0);
    Color::from_argb(RP_COLORS[usize::from(team) % 6]).with_alpha(255).scaled(k * 0.9 + 0.1)
}

/// Everything a frame draws of `state`, with each instance's owner. `assets` hands out the
/// handles; `lib` gives the library objects' boxes.
#[must_use]
pub fn build(state: &SceneState, lib: &Library, assets: &Assets, opts: &DrawOptions) -> Built {
    let mut out = Built::default();
    let tools = &state.tools;
    let snap: rustc_hash::FxHashSet<ObjId> =
        if tools.snap_use { tools.snap.iter().copied().collect() } else { rustc_hash::FxHashSet::default() };
    let h = &mut out.list.helpers;
    for o in state.objects.values() {
        if !(o.flags.visible && tools.part(o.class).visible) {
            continue;
        }
        let selected = state.is_selected(o.id);
        let world = gm(&o.world());
        let rp = gm(&o.xform.matrix_rp());
        let p = o.position();
        let flags = tools.part(o.class).flags;
        let mut push = |mesh, transform: Mat4, bounds: Option<Aabb>| {
            let mut i = Instance::new(mesh, transform).with_id(o.id.0);
            if selected {
                i = i.with_flags(InstanceFlags::SELECTED);
            }
            if snap.contains(&o.id) {
                i.tint = SNAP_TINT;
            }
            i.bounds = bounds;
            out.list.instances.push(i);
            out.owners.push(o.id);
        };
        match &o.data {
            ObjData::SceneObject(d) => {
                if d.reference.is_empty() {
                    continue;
                }
                let local = lib.cached(&d.reference).and_then(|l| ga(&l.bounds));
                let mesh =
                    if d.visual_mode() { assets.visual(&d.reference) } else { assets.object(&d.reference) };
                push(mesh, world, local);
                if selected {
                    let b = local.or_else(|| assets.mesh_bounds(mesh)).unwrap_or(Aabb::UNIT);
                    selection_box(h, &b, &world, Color::WHITE);
                }
            }
            ObjData::Light(d) => {
                let clr = line(if selected {
                    LIGHT_SEL
                } else if d.flags & light_flags::AFFECT_DYNAMIC != 0 {
                    LIGHT_DYN
                } else {
                    LIGHT_NORM
                });
                match d.kind {
                    LT_SPOT => {
                        let dir = gz_scene::math::rotation_matrix(o.rotation()).row(2);
                        h.spot_light(p, dir, if selected { d.range } else { VIS_RADIUS }, d.cone, clr);
                    }
                    _ => {
                        if selected {
                            line_sphere(h, p, d.range, clr, true);
                        }
                        h.point_light(p, VIS_RADIUS, clr);
                        if d.kind == LT_POINT
                            && d.flags & light_flags::POINT_FUZZY != 0
                            && let Some(f) = &d.fuzzy
                        {
                            for fp in &f.points {
                                h.point_light(rp.transform_point3(*fp), VIS_RADIUS / 6.0, clr);
                            }
                            let soft = clr.with_alpha(0x40);
                            if f.shape == 0 {
                                h.shape(
                                    ShapeKind::Sphere,
                                    rp * Mat4::from_scale(Vec3::splat(f.sphere_radius)),
                                    Some(soft),
                                    Some(soft),
                                    true,
                                );
                            } else {
                                h.shape(
                                    ShapeKind::Box,
                                    rp * Mat4::from_scale(f.box_dim * 2.0),
                                    Some(soft),
                                    Some(soft),
                                    true,
                                );
                            }
                        }
                    }
                }
                if flags & LIGHT_SHOW_NAME != 0
                    && let gz_scene::tools::ToolOpts::Light(lt) = &tools.part(ObjClass::Light).opts
                    && let Some(name) = lt.name_of(d.lcontrol)
                {
                    h.label(p, format!(" {name}"), Color::WHITE, true);
                }
            }
            ObjData::Glow(g) => {
                // the sprite is the game's texture: the editor shows where it is and how big
                romboid(h, p, VIS_RADIUS, line(GLOW_COLOR));
                h.sphere(p, g.radius, None, Some(line(GLOW_COLOR).with_alpha(110)), true);
                if selected {
                    let b = Aabb::new(p - Vec3::splat(g.radius), p + Vec3::splat(g.radius));
                    selection_box(h, &b, &Mat4::IDENTITY, Color::WHITE);
                    if flags & GLOW_DRAW_CROSS != 0 {
                        h.cross(p, g.radius, false, Color::WHITE, true);
                    }
                }
            }
            ObjData::Shape(d) => shapes(h, d, &world, fill(SHAPE_TRANSP), line(SHAPE_EDGE), selected),
            ObjData::SoundEnv(d) => shapes(h, &d.shape, &world, fill(ENV_TRANSP), line(SHAPE_EDGE), selected),
            ObjData::FogVolume(d) => {
                let transp = fill(if d.kind == FV_OCCLUSION { FOG_OCCLUSION_TRANSP } else { ENV_TRANSP });
                // `ESceneFogVolumeTool::Selected`: the volumes of a selected one's group show it
                let grouped = d.group_id != u32::MAX
                    && state
                        .selection
                        .iter()
                        .filter_map(|id| state.get(*id))
                        .any(|s| matches!(&s.data, ObjData::FogVolume(f) if f.group_id == d.group_id));
                shapes(
                    h,
                    &d.shape,
                    &world,
                    transp,
                    line(if grouped { FOG_GROUP_EDGE } else { SHAPE_EDGE }),
                    selected,
                );
            }
            ObjData::SoundSrc(d) => {
                if selected {
                    line_sphere(h, d.position, d.max_distance, line(SOUND_SEL1), true);
                    line_sphere(h, d.position, d.min_distance, line(SOUND_SEL0), false);
                } else {
                    h.cross(d.position, VIS_RADIUS, true, line(SOUND_NORM), true);
                }
            }
            ObjData::Spawn(d) => {
                if let Some(a) = &d.attached
                    && opts.helpers
                    && let ObjData::Shape(s) = &a.data
                {
                    shapes(h, s, &gm(&a.xform.matrix()), fill(SHAPE_TRANSP), line(SHAPE_EDGE), selected);
                }
                if let Some(visual) = d
                    .entity
                    .as_ref()
                    .and_then(|e| e.entity())
                    .and_then(|e| e.visual_name())
                    .filter(|v| !v.is_empty())
                {
                    let mesh = assets.visual(visual);
                    push(mesh, rp, None);
                }
                if opts.helpers {
                    match d.kind {
                        PointKind::Spawn => entity_flag(h, &rp, Color::WHITE),
                        PointKind::RPoint => entity_flag(h, &rp, rpoint_color(d.rpoint.team)),
                        PointKind::EnvMod => {
                            h.cross(p, 0.25, true, line(ENVMOD_COLOR), true);
                            if selected {
                                h.sphere(
                                    p,
                                    d.env.radius,
                                    Some(with_alpha(ENVMOD_COLOR, 0x30)),
                                    Some(line(ENVMOD_COLOR)),
                                    true,
                                );
                            }
                        }
                        PointKind::PlayerStart => {
                            let c = line(PLAYERSTART_COLOR);
                            entity_flag(h, &rp, c);
                            // the way the player faces on the first frame
                            let (tail, tip) = (Vec3::new(0.0, 0.05, 0.0), Vec3::new(0.0, 0.05, 1.2));
                            let (left, right) = (Vec3::new(-0.25, 0.05, 0.9), Vec3::new(0.25, 0.05, 0.9));
                            h.line(rp.transform_point3(tail), rp.transform_point3(tip), c, true);
                            h.line(rp.transform_point3(tip), rp.transform_point3(left), c, true);
                            h.line(rp.transform_point3(tip), rp.transform_point3(right), c, true);
                            // the player's standing room
                            let room = Aabb::new(
                                Vec3::new(-PLAYERSTART_HALF_WIDTH, 0.0, -PLAYERSTART_HALF_WIDTH),
                                Vec3::new(
                                    PLAYERSTART_HALF_WIDTH,
                                    objects::PLAYERSTART_HEIGHT,
                                    PLAYERSTART_HALF_WIDTH,
                                ),
                            );
                            h.shape(
                                ShapeKind::Box,
                                rp * room.unit_transform(),
                                None,
                                Some(c.with_alpha(120)),
                                true,
                            );
                        }
                    }
                    let spawn_flags = tools.part(ObjClass::SpawnPoint).flags;
                    if spawn_flags & SPAWN_SHOW_TYPE != 0 {
                        h.label(p, spawn_label(o, d), Color::WHITE, spawn_flags & SPAWN_PICK_TYPE != 0);
                    }
                    if selected && let Some(b) = world_box(state, lib, o) {
                        selection_box(h, &b, &Mat4::IDENTITY, Color::WHITE);
                    }
                }
            }
            ObjData::Way(w) => {
                let lift = Vec3::new(0.0, WAYPOINT_SIZE * 0.85, 0.0);
                for wp in &w.points {
                    let top = wp.position + lift;
                    cross6(
                        h,
                        top,
                        WAYPOINT_RADIUS,
                        WAYPOINT_SIZE * 0.85,
                        WAYPOINT_RADIUS,
                        WAYPOINT_RADIUS,
                        WAYPOINT_SIZE * 0.15,
                        WAYPOINT_RADIUS,
                        line(0x0000_ff00),
                    );
                    let link_c = Color::from_argb(if selected {
                        if wp.selected { 0xffff_ff00 } else { 0xff90_9000 }
                    } else {
                        0xff60_6000
                    });
                    for (to, prob) in &wp.links {
                        let Some(other) = w.points.get(*to as usize) else { continue };
                        let p2 = other.position + lift;
                        h.arrow(top, p2, 0.25, link_c, true);
                        if selected {
                            let c = Color::from_argb(if wp.selected { 0xFFFF_FFFF } else { 0xFFA0_A0A0 });
                            h.label(top + (p2 - top) * 0.95, format!("P: {prob:1.2}"), c, false);
                        }
                    }
                    if selected {
                        let c = Color::from_argb(if wp.selected { 0xFFFF_FFFF } else { 0xFFA0_A0A0 });
                        h.label(wp.position, format!(" {} [{}]", o.name, wp.name), c, false);
                        if wp.selected {
                            selection_box(
                                h,
                                &ga(&wp.bounds()).unwrap_or(Aabb::UNIT),
                                &Mat4::IDENTITY,
                                Color::WHITE,
                            );
                        }
                    }
                }
                if selected && let Some(b) = ga(&w.bounds()) {
                    selection_box(h, &b, &Mat4::IDENTITY, Color::WHITE);
                }
            }
            ObjData::Sector(s) => {
                // the sector's room: its items' box in its colour
                if let Some(b) = world_box(state, lib, o) {
                    let base = Color::rgb(
                        (s.color[0] * 255.0) as u8,
                        (s.color[1] * 255.0) as u8,
                        (s.color[2] * 255.0) as u8,
                    );
                    let k = if selected { 0.4 } else { 0.2 };
                    let edge = base.scaled(if selected { 0.8 } else { 0.5 });
                    h.aabb(&b, Some(base.with_alpha((k * 255.0) as u8)), Some(edge), true);
                }
            }
            ObjData::Portal(pd) => {
                if pd.vertices.len() < 2 {
                    continue;
                }
                let center = pd.vertices.iter().copied().sum::<Vec3>() / pd.vertices.len() as f32;
                let sector_color = |name: &str| {
                    state.find(ObjClass::Sector, name).and_then(|s| match &s.data {
                        ObjData::Sector(sd) => Some(Color::rgb(
                            (sd.color[0] * 255.0) as u8,
                            (sd.color[1] * 255.0) as u8,
                            (sd.color[2] * 255.0) as u8,
                        )),
                        _ => None,
                    })
                };
                if let Some(c) = sector_color(&pd.front).or_else(|| sector_color(&pd.back)) {
                    let c = if selected { c } else { c.scaled(0.7) };
                    for v in &pd.vertices {
                        h.line(center, *v, c, true);
                    }
                }
                let edge = if selected { Color::WHITE } else { Color::WHITE.scaled(0.5) };
                for (i, v) in pd.vertices.iter().enumerate() {
                    let next = pd.vertices.get((i + 1) % pd.vertices.len()).copied().unwrap_or(*v);
                    h.line(*v, next, edge, true);
                }
                // `DrawFaceNormal`
                let mut n = Vec3::ZERO;
                for (i, a) in pd.vertices.iter().enumerate() {
                    let b = pd.vertices.get((i + 1) % pd.vertices.len()).copied().unwrap_or(*a);
                    n += (*a - center).cross(b - center);
                }
                if let Some(n) = n.try_normalize() {
                    h.line(center, center + n, Color::WHITE, true);
                }
            }
            ObjData::Group(_) => {
                if selected && let Some(b) = world_box(state, lib, o) {
                    selection_box(h, &b, &Mat4::IDENTITY, line(GROUP_SEL));
                }
            }
            ObjData::Particles(_) => {
                let c = line(PARTICLES_COLOR);
                if !selected {
                    cross6(h, p, 0.30, 0.1, 0.3, 0.3, 0.3, 0.3, c);
                }
                romboid(h, p, 0.1, c);
                if selected && let Some(b) = world_box(state, lib, o) {
                    selection_box(h, &b, &Mat4::IDENTITY, Color::WHITE);
                }
            }
            ObjData::Kept(_) => h.cross(p, VIS_RADIUS, false, Color::rgb(160, 160, 160), true),
        }
    }
    if let Some((env, lights)) = &opts.game {
        out.list.environment = Some(env.clone());
        out.list.lights.clone_from(lights);
    }
    out
}

/// Which instances a frustum sees, from the scene's own pick tree: scene objects by the tree's
/// boxes, the rest by their own bounds (or always, while those are not known).
#[must_use]
pub fn visibility(
    built: &Built,
    state: &SceneState,
    pick: &ScenePick,
    assets: &Assets,
    frustum: &gz_render::Frustum,
    reach: &Aabb,
) -> Visibility {
    let near: rustc_hash::FxHashSet<ObjId> =
        pick.box_candidates(&gz_fmt_level::Aabb::new(reach.min, reach.max)).into_iter().collect();
    let mut visible = Vec::with_capacity(built.list.instances.len());
    for (i, (inst, owner)) in built.list.instances.iter().zip(&built.owners).enumerate() {
        let scene_object = state.get(*owner).is_some_and(|o| o.class == ObjClass::SceneObject);
        let seen = match inst.bounds.or_else(|| assets.mesh_bounds(inst.mesh)) {
            Some(local) => {
                let world = local.transformed(&inst.transform);
                (!scene_object || near.contains(owner)) && frustum.intersects_aabb(&world)
            }
            None => true,
        };
        if seen && let Ok(i) = u32::try_from(i) {
            visible.push(i);
        }
    }
    Visibility { visible }
}

/// The box a frustum reaches: its eight corners (`view_proj`'s inverse at the clip corners).
#[must_use]
pub fn frustum_reach(view_proj: &Mat4) -> Aabb {
    let inv = view_proj.inverse();
    let mut b = Aabb::EMPTY;
    for x in [-1.0f32, 1.0] {
        for y in [-1.0f32, 1.0] {
            for z in [0.0f32, 1.0] {
                let p = inv * glam::Vec4::new(x, y, z, 1.0);
                if p.w.abs() > 1e-9 {
                    b.extend(p.truncate() / p.w);
                }
            }
        }
    }
    b
}

/// The objects a box of the picture holds (`FrustumSelect`): a frustum through the rectangle
/// (pixels of a picture `size` big), tested against each object's world box.
#[must_use]
pub fn frustum_select(
    state: &SceneState,
    lib: &Library,
    camera: &gz_render::Camera,
    size: [u32; 2],
    a: glam::Vec2,
    b: glam::Vec2,
    class: ObjClass,
) -> Vec<ObjId> {
    let (w, h) = (size[0].max(1) as f32, size[1].max(1) as f32);
    let (x0, x1) = (a.x.min(b.x) / w * 2.0 - 1.0, a.x.max(b.x) / w * 2.0 - 1.0);
    let (y0, y1) = (1.0 - a.y.max(b.y) / h * 2.0, 1.0 - a.y.min(b.y) / h * 2.0);
    if (x1 - x0).abs() < 1e-6 || (y1 - y0).abs() < 1e-6 {
        return Vec::new();
    }
    // the rectangle's clip range stretched to the whole clip square
    let sx = 2.0 / (x1 - x0);
    let sy = 2.0 / (y1 - y0);
    let tx = -(x1 + x0) / (x1 - x0);
    let ty = -(y1 + y0) / (y1 - y0);
    let s = Mat4::from_cols(
        glam::Vec4::new(sx, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, sy, 0.0, 0.0),
        glam::Vec4::new(0.0, 0.0, 1.0, 0.0),
        glam::Vec4::new(tx, ty, 0.0, 1.0),
    );
    let frustum = gz_render::Frustum::from_view_proj(&(s * camera.view_proj(size)));
    state
        .of_class(class)
        .filter(|o| o.flags.visible && o.editable())
        .filter(|o| match &o.data {
            // lights, sounds, glows and particles test their mark, not their range
            ObjData::Light(_) | ObjData::SoundSrc(_) => frustum.intersects_sphere(o.position(), VIS_RADIUS),
            ObjData::Glow(g) => frustum.intersects_sphere(o.position(), g.radius),
            ObjData::Particles(_) => frustum.intersects_sphere(o.position(), objects::PSOBJECT_SIZE),
            _ => world_box(state, lib, o).is_some_and(|b| frustum.intersects_aabb(&b)),
        })
        .map(|o| o.id)
        .collect()
}

/// The box Frame All and Frame Selection fit (`EScene::ZoomExtents` of the current class).
#[must_use]
pub fn class_box(state: &SceneState, lib: &Library, class: ObjClass, selected_only: bool) -> Option<Aabb> {
    let mut b = Aabb::EMPTY;
    for o in state.of_class(class) {
        if !o.flags.visible || (selected_only && !state.is_selected(o.id)) {
            continue;
        }
        if let Some(ob) = world_box(state, lib, o) {
            b = b.union(&ob);
        }
    }
    (!b.is_empty()).then_some(b)
}

/// An object's box for framing (`focus_object`): its world box, or two metres round its pivot.
#[must_use]
pub fn object_box(state: &SceneState, lib: &Library, o: &Object) -> Aabb {
    world_box(state, lib, o).unwrap_or_else(|| {
        let p = o.position();
        Aabb::new(p - Vec3::splat(2.0), p + Vec3::splat(2.0))
    })
}

/// What the gizmo of the current class's selection stands on: each selected, visible, editable
/// object's pivot, rotation and box.
#[must_use]
pub fn gizmo_points(state: &SceneState, lib: &Library, class: ObjClass) -> Vec<(Vec3, Vec3, Option<Aabb>)> {
    state
        .of_class(class)
        .filter(|o| state.is_selected(o.id) && o.flags.visible && o.editable())
        .map(|o| (o.position(), o.rotation(), world_box(state, lib, o)))
        .collect()
}

/// A shared, empty list: what an unopened scene draws (the grid alone).
#[must_use]
pub fn empty() -> Arc<Built> {
    Arc::new(Built::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_scene::objects::{GlowData, LightData};

    fn scene() -> SceneState {
        let mut s = SceneState::default();
        let mut l = Object::new(ObjClass::Light, "l", ObjData::Light(LightData::default()));
        l.xform.position = Vec3::new(1.0, 2.0, 3.0);
        let lid = s.insert(l);
        s.insert(Object::new(ObjClass::Glow, "g", ObjData::Glow(GlowData::default())));
        s.selection.insert(lid);
        s
    }

    #[test]
    fn helpers_follow_the_classes() {
        let s = scene();
        let lib = Library::new(Vec::new());
        let assets = Assets::headless(gz_render::MaterialTable::default());
        let b = build(&s, &lib, &assets, &DrawOptions { helpers: true, game: None });
        // lights and glows have no mesh: only helpers
        assert!(b.list.instances.is_empty());
        assert!(!b.list.helpers.shapes.is_empty());
        assert!(!b.list.helpers.lines.is_empty());
        // the selected light draws its range: a sphere of 8 m
        assert!(
            b.list
                .helpers
                .shapes
                .iter()
                .any(|sh| sh.kind == ShapeKind::Sphere && (sh.transform.x_axis.x - 8.0).abs() < 1e-4)
        );
    }

    #[test]
    fn hidden_classes_draw_nothing() {
        let mut s = scene();
        s.tools.part_mut(ObjClass::Light).visible = false;
        s.tools.part_mut(ObjClass::Glow).visible = false;
        let lib = Library::new(Vec::new());
        let assets = Assets::headless(gz_render::MaterialTable::default());
        let b = build(&s, &lib, &assets, &DrawOptions { helpers: true, game: None });
        assert!(b.list.helpers.is_empty());
    }

    #[test]
    fn a_marquee_selects_what_it_holds() {
        let s = scene();
        let lib = Library::new(Vec::new());
        let cam = gz_render::Camera::look_at(Vec3::new(1.0, 2.0, -10.0), Vec3::new(1.0, 2.0, 3.0));
        let size = [800, 600];
        let hit = frustum_select(
            &s,
            &lib,
            &cam,
            size,
            glam::Vec2::new(350.0, 250.0),
            glam::Vec2::new(450.0, 350.0),
            ObjClass::Light,
        );
        assert_eq!(hit.len(), 1);
        let miss = frustum_select(
            &s,
            &lib,
            &cam,
            size,
            glam::Vec2::new(0.0, 0.0),
            glam::Vec2::new(50.0, 50.0),
            ObjClass::Light,
        );
        assert!(miss.is_empty());
    }

    #[test]
    fn the_reach_of_a_frustum_holds_what_it_sees() {
        let cam = gz_render::Camera::looking(Vec3::ZERO, Vec3::Z);
        let r = frustum_reach(&cam.view_proj([100, 100]));
        assert!(r.max.z > 1000.0 && r.min.z < 1.0);
    }
}
