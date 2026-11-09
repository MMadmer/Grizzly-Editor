//! `STATE_Read` and `STATE_Write` of every class.
//!
//! One function per C++ class, calling the functions of its bases in the order the C++ calls
//! `inherited::STATE_*`. Every `m_wVersion` branch is kept. Where the game's classes and the
//! editor's differ, the layout is the game's and the comment says so.

use glam::Vec3;
use smol_str::SmolStr;

use crate::class::{Class, ClassInfo};
use crate::entity::Header;
use crate::model::{Object, SavedBone, SavedBones, Shape, skeleton_flags};
use crate::packet::{NetReader, NetWriter, PacketError, PacketResult};
use crate::script::Script;
use crate::{EntityError, Result};

/// `weaponSidecarConditionSaveMask`: bits of the condition type that never reach a packet.
const WEAPON_CONDITION_SAVE_MASK: u32 = (1 << 25) | (1 << 26);

/// What a `STATE_Read` works on.
pub(crate) struct ReadCtx<'a, 'b> {
    pub o: &'a mut Object,
    pub h: &'a mut Header,
    pub info: &'a ClassInfo,
    pub r: &'a mut NetReader<'b>,
    /// `m_wVersion`.
    pub v: u16,
}

// ---------------------------------------------------------------- shared pieces

/// `load_data(xr_vector<u16>)`: a count and raw words.
fn r_u16_vec(r: &mut NetReader<'_>) -> PacketResult<Vec<u16>> {
    let n = r.r_u32()? as usize;
    let mut out = Vec::with_capacity(r.checked_count(n, 2)?);
    for _ in 0..n {
        let b = r.take(2)?;
        out.push(u16::from_le_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]));
    }
    Ok(out)
}

/// `save_data(xr_vector<u16>)`.
fn w_u16_vec(w: &mut NetWriter, items: &[u16]) {
    w.w_u32(items.len() as u32);
    for v in items {
        w.w(&v.to_le_bytes());
    }
}

/// `load_data` of a vector of raw bytes: a count and the bytes.
fn r_bytes_vec<'a>(r: &mut NetReader<'a>) -> PacketResult<&'a [u8]> {
    let n = r.r_u32()? as usize;
    if n == 0 {
        return Ok(&[]);
    }
    r.take(n)
}

fn w_bytes_vec(w: &mut NetWriter, items: &[u8]) {
    w.w_u32(items.len() as u32);
    w.w(items);
}

fn r_string_vec(r: &mut NetReader<'_>) -> PacketResult<Vec<SmolStr>> {
    let n = r.r_u32()? as usize;
    let mut out = Vec::with_capacity(r.checked_count(n, 1)?);
    for _ in 0..n {
        out.push(r.r_stringz()?);
    }
    Ok(out)
}

/// `CSE_Visual::visual_read`.
fn r_visual(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    c.o.visual.name = c.r.r_stringz()?;
    if c.v > 103 {
        c.o.visual.flags = c.r.r_u8()?;
    }
    Ok(())
}

/// `CSE_Visual::visual_write`.
fn w_visual(o: &Object, w: &mut NetWriter) {
    w.w_stringz(&o.visual.name);
    w.w_u8(o.visual.flags);
}

/// `CSE_Shape::cform_read`.
fn r_cform(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    c.o.shapes.clear();
    let count = c.r.r_u8()?;
    for _ in 0..count {
        let shape = match c.r.r_u8()? {
            // raw `Fsphere` in a binary packet, a vector and a float in a scene section:
            // the same sixteen bytes either way
            0 => Shape::Sphere { center: c.r.r_vec3()?, radius: c.r.r_float()? },
            1 => {
                let [i, j, k, pos] = c.r.r_matrix()?;
                Shape::Box { i, j, k, c: pos }
            }
            other => Shape::Other(other),
        };
        c.o.shapes.push(shape);
    }
    Ok(())
}

/// `CSE_Shape::cform_write`.
fn w_cform(o: &Object, w: &mut NetWriter) {
    w.w_u8(o.shapes.len() as u8);
    for shape in &o.shapes {
        match *shape {
            Shape::Sphere { center, radius } => {
                w.w_u8(0);
                w.w_vec3(center);
                w.w_float(radius);
            }
            Shape::Box { i, j, k, c } => {
                w.w_u8(1);
                w.w_matrix(&[i, j, k, c]);
            }
            Shape::Other(ty) => w.w_u8(ty),
        }
    }
}

/// `SPHBonesData::net_Load`. The game's layout: one 64-bit bone mask (the editor's engine
/// carries four).
fn r_saved_bones(r: &mut NetReader<'_>) -> PacketResult<SavedBones> {
    let bones_mask = r.r_u64()?;
    let root_bone = r.r_u16()?;
    let min = r.r_vec3()?;
    let max = r.r_vec3()?;
    let n = usize::from(r.r_u16()?);
    let mut bones = Vec::with_capacity(r.checked_count(n, 8)?);
    for _ in 0..n {
        let position = [r.r_u8()?, r.r_u8()?, r.r_u8()?];
        let rotation = [r.r_u8()?, r.r_u8()?, r.r_u8()?, r.r_u8()?];
        bones.push(SavedBone { position, rotation, enabled: r.r_u8()? != 0 });
    }
    Ok(SavedBones { bones_mask, root_bone, min, max, bones })
}

/// `SPHBonesData::net_Save`.
fn w_saved_bones(b: &SavedBones, w: &mut NetWriter) {
    w.w_u64(b.bones_mask);
    w.w_u16(b.root_bone);
    w.w_vec3(b.min);
    w.w_vec3(b.max);
    w.w_u16(b.bones.len() as u16);
    for bone in &b.bones {
        for q in bone.position {
            w.w_u8(q);
        }
        for q in bone.rotation {
            w.w_u8(q);
        }
        w.w_u8(u8::from(bone.enabled));
    }
}

/// `CSE_PHSkeleton::data_load`, virtual: `CSE_ALifeCar` reads its own state after the bones.
fn r_skeleton_data(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    c.o.skeleton.saved = Some(Box::new(r_saved_bones(c.r)?));
    c.o.skeleton.flags |= skeleton_flags::SAVED_DATA;
    if c.info.class == Class::Car {
        c.h.position = c.r.r_vec3()?;
        c.h.angle = c.r.r_vec3()?;
        let n = usize::from(c.r.r_u16()?);
        let mut doors = Vec::with_capacity(c.r.checked_count(n, 5)?);
        for _ in 0..n {
            doors.push((c.r.r_u8()?, c.r.r_float()?));
        }
        let n = usize::from(c.r.r_u16()?);
        let mut wheels = Vec::with_capacity(c.r.checked_count(n, 4)?);
        for _ in 0..n {
            wheels.push(c.r.r_float()?);
        }
        let health = c.r.r_float()?;
        let car = c.o.car_mut();
        car.doors = doors;
        car.wheels = wheels;
        car.health = health;
    }
    Ok(())
}

/// `CSE_PHSkeleton::data_save`.
fn w_skeleton_data(o: &Object, h: &Header, class: Class, w: &mut NetWriter) {
    match &o.skeleton.saved {
        Some(bones) => w_saved_bones(bones, w),
        None => w_saved_bones(&SavedBones::default(), w),
    }
    if class == Class::Car {
        w.w_vec3(h.position);
        w.w_vec3(h.angle);
        let (doors, wheels, health) = o.car().map_or((&[][..], &[][..], 1.0), |c| (&c.doors[..], &c.wheels[..], c.health));
        w.w_u16(doors.len() as u16);
        for (open_state, health) in doors {
            w.w_u8(*open_state);
            w.w_float(*health);
        }
        w.w_u16(wheels.len() as u16);
        for health in wheels {
            w.w_float(*health);
        }
        w.w_float(health);
    }
}

/// `CSE_PHSkeleton::STATE_Read`.
fn r_skeleton(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    c.o.visual.startup_animation = c.r.r_stringz()?;
    c.o.skeleton.flags = c.r.r_u8()?;
    c.o.skeleton.source_id = c.r.r_u16()?;
    if c.o.skeleton.flags & skeleton_flags::SAVED_DATA != 0 {
        r_skeleton_data(c)?;
    }
    Ok(())
}

/// `CSE_PHSkeleton::STATE_Write`.
fn w_skeleton(o: &Object, h: &Header, class: Class, w: &mut NetWriter) {
    w.w_stringz(&o.visual.startup_animation);
    w.w_u8(o.skeleton.flags);
    w.w_u16(o.skeleton.source_id);
    if o.skeleton.flags & skeleton_flags::SAVED_DATA != 0 {
        w_skeleton_data(o, h, class, w);
    }
}

// ---------------------------------------------------------------- xrServer_Objects_ALife

/// `CSE_ALifeObject::STATE_Read` (and `CSE_ALifeDynamicObject`, which adds nothing).
fn r_alife(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    let v = c.v;
    let r = &mut *c.r;
    let a = &mut c.o.alife;
    if v >= 1 {
        if v > 24 {
            if v < 83 {
                r.r_float()?;
            }
        } else {
            r.r_u8()?;
        }
        if v < 83 {
            r.r_u32()?;
        }
        if v < 4 {
            r.r_u16()?;
        }
        a.graph_id = r.r_u16()?;
        a.distance = r.r_float()?;
    }
    if v >= 4 {
        a.direct_control = r.r_u32()? != 0;
    }
    if v >= 8 {
        a.node_id = r.r_u32()?;
    }
    if v > 22 && v <= 79 {
        c.h.spawn_id = r.r_u16()?;
    }
    if v > 23 && v < 84 {
        r.skip_stringz()?;
    }
    if v > 49 {
        a.flags = r.r_u32()?;
    }
    if v > 57 {
        a.ini_string = r.r_stringz()?;
    }
    if v > 61 {
        a.story_id = r.r_u32()?;
    }
    if v > 111 {
        a.spawn_story_id = r.r_u32()?;
    }
    Ok(())
}

/// `CSE_ALifeObject::STATE_Write`.
fn w_alife(o: &Object, w: &mut NetWriter) {
    let a = &o.alife;
    w.w_u16(a.graph_id);
    w.w_float(a.distance);
    w.w_u32(u32::from(a.direct_control));
    w.w_u32(a.node_id);
    w.w_u32(a.flags);
    w.w_stringz(&a.ini_string);
    w.w_u32(a.story_id);
    w.w_u32(a.spawn_story_id);
}

/// `CSE_ALifeDynamicObjectVisual::STATE_Read`.
fn r_dyn_visual(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_alife(c)?;
    if c.v > 31 {
        r_visual(c)?;
    }
    Ok(())
}

fn w_dyn_visual(o: &Object, w: &mut NetWriter) {
    w_alife(o, w);
    w_visual(o, w);
}

/// `CSE_ALifeSpaceRestrictor::STATE_Read`.
fn r_space_restrictor(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_alife(c)?;
    r_cform(c)?;
    if c.v > 74 {
        c.o.restrictor_mut().restrictor_type = c.r.r_u8()?;
    }
    Ok(())
}

fn w_space_restrictor(o: &Object, w: &mut NetWriter) {
    w_alife(o, w);
    w_cform(o, w);
    w.w_u8(o.restrictor().map_or(0, |r| r.restrictor_type));
}

/// `CSE_ALifeLevelChanger::STATE_Read`.
fn r_level_changer(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_space_restrictor(c)?;
    let v = c.v;
    let r = &mut *c.r;
    let lc = c.o.level_changer_mut();
    if v < 34 {
        r.r_u32()?;
        r.r_u32()?;
    } else {
        lc.next_graph_id = r.r_u16()?;
        lc.next_node_id = r.r_u32()?;
        lc.next_position = Vec3::new(r.r_float()?, r.r_float()?, r.r_float()?);
        lc.angles = if v <= 53 { Vec3::new(0.0, r.r_float()?, 0.0) } else { r.r_vec3()? };
    }
    lc.level_to_change = r.r_stringz()?;
    lc.level_point_to_change = r.r_stringz()?;
    if v > 116 {
        lc.silent_mode = r.r_u8()? != 0;
    }
    Ok(())
}

fn w_level_changer(o: &Object, w: &mut NetWriter) {
    w_space_restrictor(o, w);
    let Some(lc) = o.level_changer() else { return };
    w.w_u16(lc.next_graph_id);
    w.w_u32(lc.next_node_id);
    w.w_float(lc.next_position.x);
    w.w_float(lc.next_position.y);
    w.w_float(lc.next_position.z);
    w.w_vec3(lc.angles);
    w.w_stringz(&lc.level_to_change);
    w.w_stringz(&lc.level_point_to_change);
    w.w_u8(u8::from(lc.silent_mode));
}

/// `CSE_ALifeObjectPhysic::STATE_Read`.
fn r_physic(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    let v = c.v;
    if v >= 14 {
        if v >= 16 {
            r_dyn_visual(c)?;
            if v < 32 {
                r_visual(c)?;
            }
        } else {
            r_alife(c)?;
            r_visual(c)?;
        }
    }
    if v >= 64 {
        r_skeleton(c)?;
    }
    let kind = c.r.r_u32()?;
    let mass = c.r.r_float()?;
    let p = c.o.physic_mut();
    p.kind = kind;
    p.mass = mass;
    if v > 9 {
        p.fixed_bones = c.r.r_stringz()?;
    }
    if v < 65 && v > 28 {
        c.o.visual.startup_animation = c.r.r_stringz()?;
    }
    if v < 64 {
        if v > 39 {
            c.o.skeleton.flags = c.r.r_u8()?;
        }
        if v > 56 {
            c.o.skeleton.source_id = c.r.r_u16()?;
        }
        if v > 60 && c.o.skeleton.flags & skeleton_flags::SAVED_DATA != 0 {
            r_skeleton_data(c)?;
        }
    }
    Ok(())
}

fn w_physic(o: &Object, h: &Header, class: Class, w: &mut NetWriter) {
    w_dyn_visual(o, w);
    w_skeleton(o, h, class, w);
    let Some(p) = o.physic() else { return };
    w.w_u32(p.kind);
    w.w_float(p.mass);
    w.w_stringz(&p.fixed_bones);
}

/// `CSE_ALifeObjectHangingLamp::STATE_Read`.
fn r_lamp(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    let v = c.v;
    if v > 20 {
        r_dyn_visual(c)?;
    }
    if v >= 69 {
        r_skeleton(c)?;
    }
    if v < 32 {
        r_visual(c)?;
    }
    let r = &mut *c.r;
    let visual = &mut c.o.visual;
    let l = c.o.lamp.get_or_insert_with(Default::default);
    if v < 49 {
        l.color = r.r_u32()?;
        l.color_animator = r.r_stringz()?;
        r.skip_stringz()?;
        r.skip_stringz()?;
        l.range = r.r_float()?;
        r.r_angle8()?;
        if v > 10 {
            l.brightness = r.r_float()?;
        }
        if v > 11 {
            l.flags = r.r_u16()?;
        }
        if v > 12 {
            r.r_float()?;
        }
        if v > 17 {
            visual.startup_animation = r.r_stringz()?;
        }
        if v > 42 {
            r.skip_stringz()?;
            r.r_float()?;
        }
        if v > 43 {
            l.fixed_bones = r.r_stringz()?;
        }
        if v > 44 {
            l.health = r.r_float()?;
        }
    } else {
        l.color = r.r_u32()?;
        l.brightness = r.r_float()?;
        l.color_animator = r.r_stringz()?;
        l.range = r.r_float()?;
        l.flags = r.r_u16()?;
        visual.startup_animation = r.r_stringz()?;
        l.fixed_bones = r.r_stringz()?;
        l.health = r.r_float()?;
    }
    if v > 55 {
        l.virtual_size = r.r_float()?;
        l.ambient_radius = r.r_float()?;
        l.ambient_power = r.r_float()?;
        l.ambient_texture = r.r_stringz()?;
        l.light_texture = r.r_stringz()?;
        l.light_main_bone = r.r_stringz()?;
        l.spot_cone_angle = r.r_float()?;
        l.glow_texture = r.r_stringz()?;
        l.glow_radius = r.r_float()?;
    }
    if v > 96 {
        l.light_ambient_bone = r.r_stringz()?;
    } else {
        l.light_ambient_bone = l.light_main_bone.clone();
    }
    if v > 118 {
        l.volumetric_quality = r.r_float()?;
        l.volumetric_intensity = r.r_float()?;
        l.volumetric_distance = r.r_float()?;
    }
    Ok(())
}

fn w_lamp(o: &Object, h: &Header, class: Class, w: &mut NetWriter) {
    w_dyn_visual(o, w);
    w_skeleton(o, h, class, w);
    let Some(l) = o.lamp() else { return };
    w.w_u32(l.color);
    w.w_float(l.brightness);
    w.w_stringz(&l.color_animator);
    w.w_float(l.range);
    w.w_u16(l.flags);
    w.w_stringz(&o.visual.startup_animation);
    w.w_stringz(&l.fixed_bones);
    w.w_float(l.health);
    w.w_float(l.virtual_size);
    w.w_float(l.ambient_radius);
    w.w_float(l.ambient_power);
    w.w_stringz(&l.ambient_texture);
    w.w_stringz(&l.light_texture);
    w.w_stringz(&l.light_main_bone);
    w.w_float(l.spot_cone_angle);
    w.w_stringz(&l.glow_texture);
    w.w_float(l.glow_radius);
    w.w_stringz(&l.light_ambient_bone);
    w.w_float(l.volumetric_quality);
    w.w_float(l.volumetric_intensity);
    w.w_float(l.volumetric_distance);
}

/// `CSE_ALifeHelicopter::STATE_Read`.
fn r_helicopter(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_dyn_visual(c)?;
    c.o.motion = c.r.r_stringz()?;
    if c.v >= 69 {
        r_skeleton(c)?;
    }
    c.o.visual.startup_animation = c.r.r_stringz()?;
    c.o.helicopter_mut().engine_sound = c.r.r_stringz()?;
    Ok(())
}

fn w_helicopter(o: &Object, h: &Header, class: Class, w: &mut NetWriter) {
    w_dyn_visual(o, w);
    w.w_stringz(&o.motion);
    w_skeleton(o, h, class, w);
    w.w_stringz(&o.visual.startup_animation);
    w.w_stringz(o.helicopter().map_or("", |h| h.engine_sound.as_str()));
}

/// `CSE_ALifeCar::STATE_Read`.
fn r_car(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_dyn_visual(c)?;
    let v = c.v;
    if v > 65 {
        r_skeleton(c)?;
    }
    if v > 52 && v < 55 {
        c.r.r_float()?;
    }
    if v > 92 {
        c.o.car_mut().health = c.r.r_float()?;
    }
    let car = c.o.car_mut();
    if car.health > 1.0 {
        car.health /= 100.0;
    }
    Ok(())
}

fn w_car(o: &Object, h: &Header, class: Class, w: &mut NetWriter) {
    w_dyn_visual(o, w);
    w_skeleton(o, h, class, w);
    w.w_float(o.car().map_or(1.0, |c| c.health));
}

/// `CSE_ALifeObjectClimable::STATE_Read`.
fn r_climable(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    if c.v >= 99 {
        r_alife(c)?;
    }
    r_cform(c)?;
    if c.v > 126 {
        c.o.climable_mut().material = c.r.r_stringz()?;
    }
    Ok(())
}

fn w_climable(o: &Object, w: &mut NetWriter) {
    w_alife(o, w);
    w_cform(o, w);
    w.w_stringz(o.climable().map_or("", |c| c.material.as_str()));
}

/// `CSE_ALifeInventoryBox::STATE_Read`.
fn r_inventory_box(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_dyn_visual(c)?;
    if c.v > 124 {
        let b = c.o.inventory_box_mut();
        b.can_take = c.r.r_u8()? == 1;
        b.closed = c.r.r_u8()? == 1;
        b.tip_text = c.r.r_stringz()?;
    }
    Ok(())
}

fn w_inventory_box(o: &Object, w: &mut NetWriter) {
    w_dyn_visual(o, w);
    let Some(b) = o.inventory_box() else { return };
    w.w_u8(u8::from(b.can_take));
    w.w_u8(u8::from(b.closed));
    w.w_stringz(&b.tip_text);
}

/// `CSE_ALifeGraphPoint::STATE_Read`.
fn r_graph_point(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    let g = c.o.graph_point_mut();
    g.connection_point_name = c.r.r_stringz()?;
    if c.v < 33 {
        c.r.r_u32()?;
    } else {
        g.connection_level_name = c.r.r_stringz()?;
    }
    for location in &mut g.locations {
        *location = c.r.r_u8()?;
    }
    Ok(())
}

fn w_graph_point(o: &Object, w: &mut NetWriter) {
    let Some(g) = o.graph_point() else { return };
    w.w_stringz(&g.connection_point_name);
    w.w_stringz(&g.connection_level_name);
    for location in g.locations {
        w.w_u8(location);
    }
}

/// `CSE_ALifeGroupAbstract::STATE_Read`.
fn r_group(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    let g = c.o.group_mut();
    g.create_spawn_positions = c.r.r_u32()? != 0;
    g.count = c.r.r_u16()?;
    if c.v > 19 {
        g.members = r_u16_vec(c.r)?;
    }
    Ok(())
}

fn w_group(o: &Object, w: &mut NetWriter) {
    let Some(g) = o.group() else { return };
    w.w_u32(u32::from(g.create_spawn_positions));
    w.w_u16(g.count);
    w_u16_vec(w, &g.members);
}

// ---------------------------------------------------------------- xrServer_Objects_ALife_Items

/// `CSE_ALifeInventoryItem::STATE_Read`.
fn r_inventory_item(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    let item = c.o.item_mut();
    if c.v > 52 {
        item.condition = c.r.r_float()?;
    }
    if c.v > 123 {
        item.upgrades = r_string_vec(c.r)?;
    }
    Ok(())
}

fn w_inventory_item(o: &Object, w: &mut NetWriter) {
    let Some(item) = o.item() else { return };
    w.w_float(item.condition);
    w.w_u32(item.upgrades.len() as u32);
    for upgrade in &item.upgrades {
        w.w_stringz(upgrade);
    }
}

/// `CSE_ALifeItem::STATE_Read`.
fn r_item(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_dyn_visual(c)?;
    if c.info.clsid == "W_BINOC " && c.v < 37 {
        c.r.r_u16()?;
        c.r.r_u16()?;
        c.r.r_u8()?;
    }
    r_inventory_item(c)
}

fn w_item(o: &Object, w: &mut NetWriter) {
    w_dyn_visual(o, w);
    w_inventory_item(o, w);
}

/// `CSE_ALifeItemWeapon::STATE_Read`, the game's: it stops quietly where the block ends, so a
/// state written by the editor's classes (no condition type) and a bare item state both load.
fn r_weapon(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_item(c)?;
    let v = c.v;
    let r = &mut *c.r;
    let can_read = |r: &NetReader<'_>, n: usize| r.is_ini() || r.r_elapsed() >= n;
    let wpn = c.o.weapon_mut();
    wpn.condition_type = None;
    if !can_read(r, 5) {
        return Ok(());
    }
    wpn.a_current = r.r_u16()?;
    wpn.a_elapsed = r.r_u16()?;
    wpn.state = r.r_u8()?;
    if v > 40 {
        if !can_read(r, 1) {
            return Ok(());
        }
        wpn.addon_flags = r.r_u8()?;
    }
    if v > 46 {
        if !can_read(r, 1) {
            return Ok(());
        }
        wpn.ammo_type = r.r_u8()?;
    }
    if v > 122 {
        if !can_read(r, 1) {
            return Ok(());
        }
        wpn.elapsed_grenades = r.r_u8()?;
        // a scene section holds no block length: the next value tells
        if r.is_ini() && r.r_eof() || !can_read(r, 4) {
            return Ok(());
        }
        wpn.condition_type = Some(r.r_u32()? & !WEAPON_CONDITION_SAVE_MASK);
    }
    Ok(())
}

fn w_weapon(o: &Object, w: &mut NetWriter) {
    w_item(o, w);
    let Some(wpn) = o.weapon() else { return };
    w.w_u16(wpn.a_current);
    w.w_u16(wpn.a_elapsed);
    w.w_u8(wpn.state);
    w.w_u8(wpn.addon_flags);
    w.w_u8(wpn.ammo_type);
    w.w_u8(wpn.elapsed_grenades);
    if let Some(condition_type) = wpn.condition_type {
        w.w_u32(condition_type & !WEAPON_CONDITION_SAVE_MASK);
    }
}

/// `CSE_ALifeItemPDA::STATE_Read`.
fn r_pda(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_item(c)?;
    let v = c.v;
    let pda = c.o.pda_mut();
    if v > 58 {
        pda.original_owner = c.r.r_u16()?;
    }
    if v > 89 {
        if v < 98 {
            c.r.take(4)?;
            c.r.take(4)?;
            pda.info_portion = SmolStr::default();
            pda.specific_character = SmolStr::default();
        } else {
            pda.specific_character = c.r.r_stringz()?;
            pda.info_portion = c.r.r_stringz()?;
        }
    }
    Ok(())
}

fn w_pda(o: &Object, w: &mut NetWriter) {
    w_item(o, w);
    let Some(pda) = o.pda() else { return };
    w.w_u16(pda.original_owner);
    // the game writes both; the editor's classes write two empty strings
    w.w_stringz(&pda.specific_character);
    w.w_stringz(&pda.info_portion);
}

// ---------------------------------------------------------------- xrServer_Objects_ALife_Monsters

/// `CSE_ALifeTraderAbstract::STATE_Read`.
fn r_trader_abstract(c: &mut ReadCtx<'_, '_>) -> Result<()> {
    let v = c.v;
    let r = &mut *c.r;
    let t = c.o.trader_mut();
    if v > 19 {
        if v < 108 {
            r.r_u32()?;
        }
        if v < 36 {
            r_u16_vec(r)?;
        }
        if v > 62 {
            t.money = r.r_u32()?;
        }
        if v > 75 && v < 98 {
            // an index into the specific characters of that old game: only "none" translates
            if r.r_s32()? != -1 {
                return Err(EntityError::Unsupported("a specific character stored as an index"));
            }
            t.specific_character = SmolStr::default();
        } else if v >= 98 {
            t.specific_character = r.r_stringz()?;
        }
        if v > 77 {
            t.flags = r.r_u32()?;
        }
        if v > 81 && v < 96 {
            return Err(EntityError::Unsupported("a character profile stored as an index"));
        } else if v > 95 {
            t.character_profile = r.r_stringz()?;
        }
        if v > 85 {
            t.community_index = r.r_s32()?;
        }
        if v > 86 {
            t.rank = r.r_s32()?;
            t.reputation = r.r_s32()?;
        }
        if v > 104 {
            t.character_name = r.r_stringz()?;
        }
    }
    if v > 124 {
        t.deadbody_can_take = r.r_u8()? == 1;
        t.deadbody_closed = r.r_u8()? == 1;
    }
    Ok(())
}

/// `CSE_ALifeTraderAbstract::STATE_Write`, the game's: it writes the character, the community,
/// the rank and the reputation it holds, where the editor's classes write "none" for all four.
fn w_trader_abstract(o: &Object, w: &mut NetWriter) {
    let Some(t) = o.trader() else { return };
    w.w_u32(t.money);
    w.w_stringz(&t.specific_character);
    w.w_u32(t.flags);
    w.w_stringz(&t.character_profile);
    w.w_s32(t.community_index);
    w.w_s32(t.rank);
    w.w_s32(t.reputation);
    w.w_stringz(&t.character_name);
    w.w_u8(u8::from(t.deadbody_can_take));
    w.w_u8(u8::from(t.deadbody_closed));
}

/// `CSE_ALifeTrader::STATE_Read`.
fn r_trader(c: &mut ReadCtx<'_, '_>) -> Result<()> {
    r_dyn_visual(c)?;
    r_trader_abstract(c)?;
    let v = c.v;
    let r = &mut *c.r;
    if v > 35 && v < 118 {
        r.r_u32()?;
    }
    if v > 29 && v < 118 {
        let count = r.r_u32()?;
        for _ in 0..count {
            r.skip_stringz()?;
            r.r_u32()?;
            let n = r.r_u32()?;
            for _ in 0..n {
                r.skip_stringz()?;
                r.r_u32()?;
                r.r_u32()?;
            }
        }
    }
    if v > 30 && v < 118 {
        let count = r.r_u32()?;
        for _ in 0..count {
            r.skip_stringz()?;
            r.r_u32()?;
            r.r_float()?;
            r.r_float()?;
        }
    }
    Ok(())
}

/// `CSE_ALifeCustomZone::STATE_Read`.
fn r_custom_zone(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_space_restrictor(c)?;
    let v = c.v;
    let r = &mut *c.r;
    let z = c.o.zone_mut();
    // the game keeps this float (`m_maxPower`); the editor's classes drop it and write 0
    z.max_power = r.r_float()?;
    if v < 113 {
        r.r_float()?;
        r.r_u32()?;
    }
    if v > 66 && v < 118 {
        r.r_u32()?;
    }
    if v > 102 {
        z.owner_id = r.r_u32()?;
    }
    if v > 105 {
        z.enabled_time = r.r_u32()?;
        z.disabled_time = r.r_u32()?;
    }
    if v > 106 {
        z.start_time_shift = r.r_u32()?;
    }
    Ok(())
}

fn w_custom_zone(o: &Object, w: &mut NetWriter) {
    w_space_restrictor(o, w);
    let Some(z) = o.zone() else { return };
    w.w_float(z.max_power);
    w.w_u32(z.owner_id);
    w.w_u32(z.enabled_time);
    w.w_u32(z.disabled_time);
    w.w_u32(z.start_time_shift);
}

/// `CSE_ALifeAnomalousZone::STATE_Read`.
fn r_anomalous_zone(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_custom_zone(c)?;
    let v = c.v;
    let r = &mut *c.r;
    let z = c.o.zone_mut();
    if v > 21 {
        z.offline_interactive_radius = r.r_float()?;
        if v < 113 {
            r.r_float()?;
            let n = r.r_u16()?;
            for _ in 0..n {
                r.skip_stringz()?;
                if v > 26 {
                    r.r_float()?;
                } else {
                    r.r_u32()?;
                }
            }
        }
    }
    if v > 25 {
        z.artefact_spawn_count = r.r_u16()?;
        z.artefact_position_offset = r.r_u32()?;
    }
    if v < 67 && v > 27 {
        r.r_u32()?;
    }
    if v > 38 && v < 113 {
        r.r_float()?;
    }
    if v > 78 && v < 113 {
        r.r_float()?;
        r.r_float()?;
        r.r_float()?;
    }
    if v == 102 {
        r.r_u32()?;
    }
    Ok(())
}

fn w_anomalous_zone(o: &Object, w: &mut NetWriter) {
    w_custom_zone(o, w);
    let Some(z) = o.zone() else { return };
    w.w_float(z.offline_interactive_radius);
    w.w_u16(z.artefact_spawn_count);
    w.w_u32(z.artefact_position_offset);
}

/// `CSE_ALifeCreatureAbstract::STATE_Read`.
fn r_creature(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_dyn_visual(c)?;
    let v = c.v;
    let cr = c.o.creature_mut();
    cr.team = c.r.r_u8()?;
    cr.squad = c.r.r_u8()?;
    cr.group = c.r.r_u8()?;
    if v > 18 {
        cr.health = c.r.r_float()?;
    }
    if v < 115 {
        cr.health /= 100.0;
    }
    if v < 32 {
        r_visual(c)?;
    }
    let angle = c.h.angle;
    let cr = c.o.creature_mut();
    cr.o_model = cr.torso_yaw;
    if v > 87 {
        cr.out_restrictions = r_u16_vec(c.r)?;
        cr.in_restrictions = r_u16_vec(c.r)?;
    }
    if v > 94 {
        cr.killer_id = c.r.r_u16()?;
    }
    cr.torso_pitch = angle.x;
    cr.torso_yaw = angle.y;
    if v > 115 {
        cr.game_death_time = c.r.r_u64()?;
    }
    Ok(())
}

fn w_creature(o: &Object, w: &mut NetWriter) {
    w_dyn_visual(o, w);
    let Some(cr) = o.creature() else { return };
    w.w_u8(cr.team);
    w.w_u8(cr.squad);
    w.w_u8(cr.group);
    w.w_float(cr.health);
    w_u16_vec(w, &cr.out_restrictions);
    w_u16_vec(w, &cr.in_restrictions);
    w.w_u16(cr.killer_id);
    w.w_u64(cr.game_death_time);
}

/// `CSE_ALifeMonsterAbstract::STATE_Read`.
fn r_monster(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_creature(c)?;
    let v = c.v;
    let r = &mut *c.r;
    let m = c.o.monster_mut();
    if v > 72 {
        m.out_space_restrictors = r.r_stringz()?;
        if v > 73 {
            m.in_space_restrictors = r.r_stringz()?;
        }
    }
    if v > 111 {
        m.smart_terrain_id = r.r_u16()?;
    }
    if v > 113 {
        m.task_reached = if r.is_ini() {
            u8::from(r.r_u16()? != 0)
        } else {
            r.take(1)?.first().copied().unwrap_or(0)
        };
    }
    Ok(())
}

fn w_monster(o: &Object, w: &mut NetWriter) {
    w_creature(o, w);
    let Some(m) = o.monster() else { return };
    w.w_stringz(&m.out_space_restrictors);
    w.w_stringz(&m.in_space_restrictors);
    w.w_u16(m.smart_terrain_id);
    if w.is_ini() {
        w.w_u16(u16::from(m.task_reached != 0));
    } else {
        w.w(&[m.task_reached]);
    }
}

/// `CSE_ALifeMonsterBase::STATE_Read`.
fn r_monster_base(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_monster(c)?;
    if c.v >= 68 {
        r_skeleton(c)?;
    }
    if c.v >= 109 {
        c.o.monster_mut().spec_object_id = c.r.r_u16()?;
    }
    Ok(())
}

fn w_monster_base(o: &Object, h: &Header, class: Class, w: &mut NetWriter) {
    w_monster(o, w);
    w_skeleton(o, h, class, w);
    w.w_u16(o.monster().map_or(u16::MAX, |m| m.spec_object_id));
}

/// `CSE_ALifeMonsterRat::STATE_Read`.
fn r_rat(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_monster(c)?;
    let v = c.v;
    let r = &mut *c.r;
    let mut old_health = None;
    let rat = c.o.rat.get_or_insert_with(Default::default);
    rat.eye_fov = r.r_float()?;
    rat.eye_range = r.r_float()?;
    if v <= 5 {
        old_health = Some(r.r_float()?);
    }
    rat.min_speed = r.r_float()?;
    rat.max_speed = r.r_float()?;
    rat.attack_speed = r.r_float()?;
    rat.max_pursuit_radius = r.r_float()?;
    rat.max_home_radius = r.r_float()?;
    rat.morale_success_attack_quant = r.r_float()?;
    rat.morale_death_quant = r.r_float()?;
    rat.morale_fear_quant = r.r_float()?;
    rat.morale_restore_quant = r.r_float()?;
    rat.morale_restore_time_interval = r.r_u16()?;
    rat.morale_min_value = r.r_float()?;
    rat.morale_max_value = r.r_float()?;
    rat.morale_normal_value = r.r_float()?;
    rat.hit_power = r.r_float()?;
    rat.hit_interval = r.r_u16()?;
    rat.attack_distance = r.r_float()?;
    rat.attack_angle = r.r_float()?;
    rat.attack_success_probability = r.r_float()?;
    if let Some(health) = old_health {
        c.o.creature_mut().health = health;
    }
    r_inventory_item(c)
}

fn w_rat(o: &Object, w: &mut NetWriter) {
    w_monster(o, w);
    if let Some(rat) = o.rat() {
        w.w_float(rat.eye_fov);
        w.w_float(rat.eye_range);
        w.w_float(rat.min_speed);
        w.w_float(rat.max_speed);
        w.w_float(rat.attack_speed);
        w.w_float(rat.max_pursuit_radius);
        w.w_float(rat.max_home_radius);
        w.w_float(rat.morale_success_attack_quant);
        w.w_float(rat.morale_death_quant);
        w.w_float(rat.morale_fear_quant);
        w.w_float(rat.morale_restore_quant);
        w.w_u16(rat.morale_restore_time_interval);
        w.w_float(rat.morale_min_value);
        w.w_float(rat.morale_max_value);
        w.w_float(rat.morale_normal_value);
        w.w_float(rat.hit_power);
        w.w_u16(rat.hit_interval);
        w.w_float(rat.attack_distance);
        w.w_float(rat.attack_angle);
        w.w_float(rat.attack_success_probability);
    }
    w_inventory_item(o, w);
}

/// `CSE_ALifeMonsterZombie::STATE_Read`.
fn r_zombie(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_monster(c)?;
    let v = c.v;
    let r = &mut *c.r;
    let mut old_health = None;
    let z = c.o.zombie.get_or_insert_with(Default::default);
    z.eye_fov = r.r_float()?;
    z.eye_range = r.r_float()?;
    if v <= 5 {
        old_health = Some(r.r_float()?);
    }
    z.min_speed = r.r_float()?;
    z.max_speed = r.r_float()?;
    z.attack_speed = r.r_float()?;
    z.max_pursuit_radius = r.r_float()?;
    z.max_home_radius = r.r_float()?;
    z.hit_power = r.r_float()?;
    z.hit_interval = r.r_u16()?;
    z.attack_distance = r.r_float()?;
    z.attack_angle = r.r_float()?;
    if let Some(health) = old_health {
        c.o.creature_mut().health = health;
    }
    Ok(())
}

fn w_zombie(o: &Object, w: &mut NetWriter) {
    w_monster(o, w);
    let Some(z) = o.zombie() else { return };
    w.w_float(z.eye_fov);
    w.w_float(z.eye_range);
    w.w_float(z.min_speed);
    w.w_float(z.max_speed);
    w.w_float(z.attack_speed);
    w.w_float(z.max_pursuit_radius);
    w.w_float(z.max_home_radius);
    w.w_float(z.hit_power);
    w.w_u16(z.hit_interval);
    w.w_float(z.attack_distance);
    w.w_float(z.attack_angle);
}

/// `CALifeHumanBrain::on_state_read`.
fn r_human_brain(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    let v = c.v;
    let r = &mut *c.r;
    if v <= 19 {
        return Ok(());
    }
    if v < 110 {
        // xr_vector<u32>, then xr_vector<bool> packed 32 to a word
        let n = r.r_u32()? as usize;
        r.take(n.saturating_mul(4))?;
        let bits = r.r_u32()? as usize;
        for _ in 0..bits.div_ceil(32) {
            r.r_u32()?;
        }
    }
    if v <= 35 {
        return Ok(());
    }
    if v < 110 {
        r.skip_stringz()?;
    }
    if v < 118 {
        r_u16_vec(r)?;
    }
    if !r.is_ini() {
        let equipment = r_bytes_vec(r)?;
        let weapons = r_bytes_vec(r)?;
        let human = c.o.human_mut();
        human.equipment_preferences = smallvec::SmallVec::from_slice(equipment);
        human.weapon_preferences = smallvec::SmallVec::from_slice(weapons);
    }
    Ok(())
}

/// `CSE_ALifeHumanAbstract::STATE_Read`: the trader part comes first.
fn r_human(c: &mut ReadCtx<'_, '_>) -> Result<()> {
    r_trader_abstract(c)?;
    r_monster(c)?;
    r_human_brain(c)?;
    if c.v >= 110 && c.v < 112 {
        let b = c.r.take(2)?;
        c.o.monster_mut().smart_terrain_id =
            u16::from_le_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]);
    }
    Ok(())
}

fn w_human(o: &Object, w: &mut NetWriter) {
    w_trader_abstract(o, w);
    w_monster(o, w);
    // `CALifeHumanBrain::on_state_write`: not for a scene section
    if !w.is_ini() {
        if let Some(human) = o.human() {
            w_bytes_vec(w, &human.equipment_preferences);
            w_bytes_vec(w, &human.weapon_preferences);
        }
    }
}

/// `CSE_ALifeHumanStalker::STATE_Read`.
fn r_stalker(c: &mut ReadCtx<'_, '_>) -> Result<()> {
    r_human(c)?;
    if c.v > 67 {
        r_skeleton(c)?;
    }
    if c.v > 90 && c.v < 111 {
        c.r.r_u8()?;
    }
    Ok(())
}

/// `CSE_ALifeCreatureActor::STATE_Read`.
fn r_actor(c: &mut ReadCtx<'_, '_>) -> Result<()> {
    let v = c.v;
    if v < 21 {
        r_alife(c)?;
        let cr = c.o.creature_mut();
        cr.team = c.r.r_u8()?;
        cr.squad = c.r.r_u8()?;
        cr.group = c.r.r_u8()?;
        if v > 18 {
            cr.health = c.r.r_float()?;
        }
        if v >= 3 {
            r_visual(c)?;
        }
    } else {
        r_creature(c)?;
        r_trader_abstract(c)?;
        if v < 32 {
            r_visual(c)?;
        }
    }
    if v > 91 {
        r_skeleton(c)?;
    }
    if v > 88 {
        c.o.actor_mut().holder_id = c.r.r_u16()?;
    }
    Ok(())
}

fn w_actor(o: &Object, h: &Header, class: Class, w: &mut NetWriter) {
    w_creature(o, w);
    w_trader_abstract(o, w);
    w_skeleton(o, h, class, w);
    w.w_u16(o.actor().map_or(u16::MAX, |a| a.holder_id));
}

/// `CSE_ALifeOnlineOfflineGroup::STATE_Read`.
fn r_squad(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_alife(c)?;
    let n = c.r.r_u32()? as usize;
    // the game: a count the payload cannot hold, or an absurd one, leaves the roster empty
    if !c.r.is_ini() && (n > c.r.r_elapsed() / 2 || n > 1024) {
        return Ok(());
    }
    let mut members = Vec::with_capacity(c.r.checked_count(n, 2)?);
    for _ in 0..n {
        let b = c.r.take(2)?;
        members.push(u16::from_le_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]));
    }
    // an associative vector: ordered by id, each id once
    members.sort_unstable();
    members.dedup();
    c.o.squad_mut().members = members;
    Ok(())
}

fn w_squad(o: &Object, w: &mut NetWriter) {
    w_alife(o, w);
    let members = o.squad().map_or(&[][..], |s| &s.members[..]);
    w.w_u32(members.len() as u32);
    for id in members {
        w.w(&id.to_le_bytes());
    }
}

/// `CSE_SmartCover::STATE_Read`.
fn r_smart_cover(c: &mut ReadCtx<'_, '_>) -> PacketResult<()> {
    r_alife(c)?;
    r_cform(c)?;
    let v = c.v;
    let r = &mut *c.r;
    let cover = c.o.cover_mut();
    cover.description = r.r_stringz()?;
    cover.hold_position_time = r.r_float()?;
    if v >= 120 {
        cover.enter_min_enemy_distance = r.r_float()?;
        cover.exit_min_enemy_distance = r.r_float()?;
    }
    if v >= 122 {
        cover.is_combat_cover = r.r_u8()?;
    }
    if v >= 128 {
        cover.can_fire = r.r_u8()?;
    } else {
        // the game: an older cover can fire when it is a combat cover
        cover.can_fire = cover.is_combat_cover;
    }
    Ok(())
}

fn w_smart_cover(o: &Object, w: &mut NetWriter) {
    w_alife(o, w);
    w_cform(o, w);
    let Some(cover) = o.cover() else { return };
    w.w_stringz(&cover.description);
    w.w_float(cover.hold_position_time);
    w.w_float(cover.enter_min_enemy_distance);
    w.w_float(cover.exit_min_enemy_distance);
    w.w_u8(cover.is_combat_cover);
    w.w_u8(cover.can_fire);
}

// ---------------------------------------------------------------- dispatch

/// `STATE_Read` of the entity's class, then the script class's fields.
pub(crate) fn state_read(c: &mut ReadCtx<'_, '_>) -> Result<()> {
    let v = c.v;
    match c.info.class {
        Class::Spectator => {}
        Class::Temporary => c.o.temporary_mut().node_id = c.r.r_u32()?,
        Class::GraphPoint => r_graph_point(c)?,
        Class::AbstractVisual => {
            r_visual(c)?;
            c.o.visual.startup_animation = c.r.r_stringz()?;
        }
        Class::DynamicObjectVisual | Class::Projector | Class::MountedWeapon | Class::StationaryMgun => {
            r_dyn_visual(c)?;
        }
        Class::PhSkeletonObject => {
            r_dyn_visual(c)?;
            if v >= 64 {
                r_skeleton(c)?;
            }
        }
        Class::SpaceRestrictor | Class::SmartZone => r_space_restrictor(c)?,
        Class::LevelChanger => r_level_changer(c)?,
        Class::ObjectPhysic => r_physic(c)?,
        Class::HangingLamp => r_lamp(c)?,
        Class::Helicopter => r_helicopter(c)?,
        Class::Car => r_car(c)?,
        Class::Breakable => {
            r_dyn_visual(c)?;
            c.o.breakable_mut().health = c.r.r_float()?;
        }
        Class::Climable => r_climable(c)?,
        Class::TeamBaseZone => {
            r_space_restrictor(c)?;
            c.o.team_base_mut().team = c.r.r_u8()?;
        }
        Class::InventoryBox => r_inventory_box(c)?,
        Class::Item
        | Class::ItemArtefact
        | Class::ItemGrenade
        | Class::ItemExplosive
        | Class::ItemBolt
        | Class::ItemCustomOutfit
        | Class::ItemHelmet => r_item(c)?,
        Class::ItemTorch | Class::ItemDetector => {
            if v > 20 {
                r_item(c)?;
            }
        }
        Class::ItemAmmo => {
            r_item(c)?;
            c.o.ammo_mut().elapsed = c.r.r_u16()?;
        }
        Class::ItemWeapon
        | Class::ItemWeaponMagazined
        | Class::ItemWeaponMagazinedWGL
        | Class::ItemWeaponShotGun
        | Class::ItemWeaponAutoShotGun => r_weapon(c)?,
        Class::ItemPda => r_pda(c)?,
        Class::ItemDocument => {
            r_item(c)?;
            if v < 98 {
                c.r.r_u16()?;
                c.o.document_mut().doc = SmolStr::default();
            } else {
                c.o.document_mut().doc = c.r.r_stringz()?;
            }
        }
        Class::Trader => r_trader(c)?,
        Class::CustomZone => r_custom_zone(c)?,
        Class::AnomalousZone => r_anomalous_zone(c)?,
        Class::TorridZone => {
            r_custom_zone(c)?;
            c.o.motion = c.r.r_stringz()?;
        }
        Class::ZoneVisual => {
            r_anomalous_zone(c)?;
            r_visual(c)?;
            c.o.visual.startup_animation = c.r.r_stringz()?;
            c.o.zone_mut().attack_animation = c.r.r_stringz()?;
        }
        Class::CreatureActor => r_actor(c)?,
        Class::CreatureCrow => {
            if v > 20 {
                r_creature(c)?;
                if v < 32 {
                    r_visual(c)?;
                }
            }
        }
        Class::CreaturePhantom => r_creature(c)?,
        Class::MonsterRat => r_rat(c)?,
        Class::MonsterZombie => r_zombie(c)?,
        Class::MonsterBase | Class::PsyDogPhantom => r_monster_base(c)?,
        Class::MonsterBaseGroup => {
            r_monster_base(c)?;
            r_group(c)?;
        }
        Class::HumanStalker => r_stalker(c)?,
        Class::OnlineOfflineGroup => r_squad(c)?,
        Class::SmartCover => r_smart_cover(c)?,
    }
    let description = c.o.cover().map(|cover| cover.description.clone()).unwrap_or_default();
    c.o.script = Script::read(c.info.script, c.r, c.h.script_version, &description)?;
    Ok(())
}

/// `STATE_Write` of the entity's class, then the script class's fields.
pub(crate) fn state_write(o: &Object, h: &Header, class: Class, w: &mut NetWriter) {
    match class {
        Class::Spectator => {}
        Class::Temporary => w.w_u32(o.temporary().map_or(u32::MAX, |t| t.node_id)),
        Class::GraphPoint => w_graph_point(o, w),
        Class::AbstractVisual => {
            w_visual(o, w);
            w.w_stringz(&o.visual.startup_animation);
        }
        Class::DynamicObjectVisual | Class::Projector | Class::MountedWeapon | Class::StationaryMgun => {
            w_dyn_visual(o, w);
        }
        Class::PhSkeletonObject => {
            w_dyn_visual(o, w);
            w_skeleton(o, h, class, w);
        }
        Class::SpaceRestrictor | Class::SmartZone => w_space_restrictor(o, w),
        Class::LevelChanger => w_level_changer(o, w),
        Class::ObjectPhysic => w_physic(o, h, class, w),
        Class::HangingLamp => w_lamp(o, h, class, w),
        Class::Helicopter => w_helicopter(o, h, class, w),
        Class::Car => w_car(o, h, class, w),
        Class::Breakable => {
            w_dyn_visual(o, w);
            w.w_float(o.breakable().map_or(1.0, |b| b.health));
        }
        Class::Climable => w_climable(o, w),
        Class::TeamBaseZone => {
            w_space_restrictor(o, w);
            w.w_u8(o.team_base().map_or(0, |t| t.team));
        }
        Class::InventoryBox => w_inventory_box(o, w),
        Class::Item
        | Class::ItemArtefact
        | Class::ItemGrenade
        | Class::ItemExplosive
        | Class::ItemBolt
        | Class::ItemCustomOutfit
        | Class::ItemHelmet
        | Class::ItemTorch
        | Class::ItemDetector => w_item(o, w),
        Class::ItemAmmo => {
            w_item(o, w);
            w.w_u16(o.ammo().map_or(0, |a| a.elapsed));
        }
        Class::ItemWeapon
        | Class::ItemWeaponMagazined
        | Class::ItemWeaponMagazinedWGL
        | Class::ItemWeaponShotGun
        | Class::ItemWeaponAutoShotGun => w_weapon(o, w),
        Class::ItemPda => w_pda(o, w),
        Class::ItemDocument => {
            w_item(o, w);
            w.w_stringz(o.document().map_or("", |d| d.doc.as_str()));
        }
        Class::Trader => {
            w_dyn_visual(o, w);
            w_trader_abstract(o, w);
        }
        Class::CustomZone => w_custom_zone(o, w),
        Class::AnomalousZone => w_anomalous_zone(o, w),
        Class::TorridZone => {
            w_custom_zone(o, w);
            w.w_stringz(&o.motion);
        }
        Class::ZoneVisual => {
            w_anomalous_zone(o, w);
            w_visual(o, w);
            w.w_stringz(&o.visual.startup_animation);
            w.w_stringz(o.zone().map_or("", |z| z.attack_animation.as_str()));
        }
        Class::CreatureActor => w_actor(o, h, class, w),
        Class::CreatureCrow | Class::CreaturePhantom => w_creature(o, w),
        Class::MonsterRat => w_rat(o, w),
        Class::MonsterZombie => w_zombie(o, w),
        Class::MonsterBase | Class::PsyDogPhantom => w_monster_base(o, h, class, w),
        Class::MonsterBaseGroup => {
            w_monster_base(o, h, class, w);
            w_group(o, w);
        }
        Class::HumanStalker => {
            w_human(o, w);
            w_skeleton(o, h, class, w);
        }
        Class::OnlineOfflineGroup => w_squad(o, w),
        Class::SmartCover => w_smart_cover(o, w),
    }
    o.script.write(w);
}

impl From<PacketError> for EntityError {
    fn from(e: PacketError) -> Self {
        Self::Packet(e)
    }
}
