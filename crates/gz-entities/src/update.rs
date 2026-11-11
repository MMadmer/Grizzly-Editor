//! `UPDATE_Read` and `UPDATE_Write` of every class.
//!
//! The editor only ever writes an update packet: `all.spawn` stores one beside each spawn
//! packet, made from the entity's state at that moment. So fields the state also carries
//! (health, position, team, condition, ammunition) are written from the state, and a read
//! takes only the fields that exist in the update alone.
//!
//! oracle: `UPDATE_Read` assigns those shared fields too. Nothing in the editor calls it, and
//! doing so would make a read spawn packet write back differently (a weapon's condition comes
//! back from eight bits). The game does it when it loads `all.spawn`, which is its business.

use glam::Vec3;

use crate::class::{Class, ClassInfo};
use crate::entity::Header;
use crate::model::{AliveState, Object, SyncState};
use crate::packet::{NetReader, NetWriter, PacketResult};

const EPS_S: f32 = 0.000_000_1;
const PI_MUL_2: f32 = 6.283_185_5;
const SYNC_ENABLED: u8 = 1 << 0;
const SYNC_ANGULAR_NULL: u8 = 1 << 1;
const SYNC_LINEAR_NULL: u8 = 1 << 2;
const WEAPON_CONDITION_SAVE_MASK: u32 = (1 << 25) | (1 << 26);

fn square_magnitude(v: Vec3) -> f32 {
    v.x * v.x + v.y * v.y + v.z * v.z
}

/// The synchronisation block of `CSE_ALifeInventoryItem` and `CSE_ALifeObjectPhysic`.
fn w_sync(sync: Option<&SyncState>, w: &mut NetWriter) {
    let Some(s) = sync.filter(|s| s.num_items != 0) else {
        w.w_u8(0);
        return;
    };
    let mut mask = 0u8;
    if s.enabled {
        mask |= SYNC_ENABLED;
    }
    if square_magnitude(s.angular_vel).abs() < EPS_S {
        mask |= SYNC_ANGULAR_NULL;
    }
    if square_magnitude(s.linear_vel).abs() < EPS_S {
        mask |= SYNC_LINEAR_NULL;
    }
    // `mask_num_items`: five bits of count, three of mask
    w.w_u8((s.num_items & 0x1f) | (mask << 5));
    w.w_vec3(s.force);
    w.w_vec3(s.torque);
    w.w_vec3(s.position);
    for q in s.quaternion {
        w.w_float(q);
    }
    if mask & SYNC_ANGULAR_NULL == 0 {
        w.w_float(s.angular_vel.x);
        w.w_float(s.angular_vel.y);
        w.w_float(s.angular_vel.z);
    }
    if mask & SYNC_LINEAR_NULL == 0 {
        w.w_float(s.linear_vel.x);
        w.w_float(s.linear_vel.y);
        w.w_float(s.linear_vel.z);
    }
    // "not freezed - doesn't mean anything"
    w.w_u8(1);
}

/// Reads the block. `quantised` is the game's branch for an item older than version 122.
fn r_sync(r: &mut NetReader<'_>, quantised: bool) -> PacketResult<Option<Box<SyncState>>> {
    let common = r.r_u8()?;
    if common == 0 {
        return Ok(None);
    }
    let mask = common >> 5;
    let mut s = SyncState { num_items: common & 0x1f, ..SyncState::default() };
    if !quantised {
        s.force = r.r_vec3()?;
        s.torque = r.r_vec3()?;
    }
    s.position = r.r_vec3()?;
    for q in &mut s.quaternion {
        *q = if quantised { r.r_float_q8(0.0, 1.0)? } else { r.r_float()? };
    }
    s.enabled = mask & SYNC_ENABLED != 0;
    if mask & SYNC_ANGULAR_NULL == 0 {
        s.angular_vel = if quantised {
            let max = 10.0 * PI_MUL_2;
            Vec3::new(r.r_float_q8(0.0, max)?, r.r_float_q8(0.0, max)?, r.r_float_q8(0.0, max)?)
        } else {
            Vec3::new(r.r_float()?, r.r_float()?, r.r_float()?)
        };
    }
    if mask & SYNC_LINEAR_NULL == 0 {
        s.linear_vel = if quantised {
            Vec3::new(r.r_float_q8(-32.0, 32.0)?, r.r_float_q8(-32.0, 32.0)?, r.r_float_q8(-32.0, 32.0)?)
        } else {
            Vec3::new(r.r_float()?, r.r_float()?, r.r_float()?)
        };
    }
    if !r.r_eof() && !quantised {
        // the freeze flag
        r.r_u8()?;
    }
    Ok(Some(Box::new(s)))
}

/// `CSE_ALifeInventoryItem::UPDATE_Read`, the game's.
fn r_item(o: &mut Object, h: &Header, r: &mut NetReader<'_>) -> PacketResult<()> {
    let sync = r_sync(r, h.version < 122)?;
    o.item_mut().sync = sync;
    Ok(())
}

fn w_item(o: &Object, w: &mut NetWriter) {
    w_sync(o.item().and_then(|i| i.sync.as_deref()), w);
}

/// `CSE_ALifeCreatureAbstract::UPDATE_Write`.
fn w_creature(o: &Object, h: &Header, w: &mut NetWriter) {
    let Some(c) = o.creature() else { return };
    w.w_float(c.health);
    w.w_u32(c.timestamp);
    w.w_u8(c.flags);
    w.w_vec3(h.position);
    w.w_float(c.o_model);
    w.w_float(c.torso_yaw);
    w.w_float(c.torso_pitch);
    w.w_float(c.torso_roll);
    w.w_u8(c.team);
    w.w_u8(c.squad);
    w.w_u8(c.group);
}

fn r_creature(o: &mut Object, r: &mut NetReader<'_>) -> PacketResult<()> {
    let c = o.creature_mut();
    r.r_float()?; // health: the state's
    c.timestamp = r.r_u32()?;
    c.flags = r.r_u8()?;
    r.r_vec3()?; // position: the header's
    c.o_model = r.r_float()?;
    c.torso_yaw = r.r_float()?;
    c.torso_pitch = r.r_float()?;
    c.torso_roll = r.r_float()?;
    r.r_u8()?; // team, squad, group: the state's
    r.r_u8()?;
    r.r_u8()?;
    Ok(())
}

/// `CSE_ALifeMonsterAbstract::UPDATE_Write`.
fn w_monster(o: &Object, h: &Header, w: &mut NetWriter) {
    w_creature(o, h, w);
    let Some(m) = o.monster() else { return };
    w.w_u16(m.next_graph_id);
    w.w_u16(m.prev_graph_id);
    w.w_float(m.distance_from_point);
    w.w_float(m.distance_to_point);
}

fn r_monster(o: &mut Object, r: &mut NetReader<'_>) -> PacketResult<()> {
    r_creature(o, r)?;
    let m = o.monster_mut();
    m.next_graph_id = r.r_u16()?;
    m.prev_graph_id = r.r_u16()?;
    m.distance_from_point = r.r_float()?;
    m.distance_to_point = r.r_float()?;
    Ok(())
}

/// `CSE_ALifeItemWeapon::UPDATE_Write`, the game's: it ends with the condition type.
fn w_weapon(o: &Object, w: &mut NetWriter) {
    w_item(o, w);
    let condition = o.item().map_or(1.0, |i| i.condition);
    let Some(wpn) = o.weapon() else { return };
    w.w_float_q8(condition, 0.0, 1.0);
    w.w_u8(wpn.wpn_flags);
    w.w_u16(wpn.a_elapsed);
    w.w_u8(wpn.addon_flags);
    w.w_u8(wpn.ammo_type);
    w.w_u8(wpn.state);
    w.w_u8(wpn.zoom);
    w.w_u32(wpn.condition_type.unwrap_or(0) & !WEAPON_CONDITION_SAVE_MASK);
}

fn r_weapon(o: &mut Object, h: &Header, r: &mut NetReader<'_>) -> PacketResult<()> {
    r_item(o, h, r)?;
    let wpn = o.weapon_mut();
    r.r_u8()?; // condition
    wpn.wpn_flags = r.r_u8()?;
    r.r_u16()?; // a_elapsed
    r.r_u8()?; // addon flags
    r.r_u8()?; // ammo type
    r.r_u8()?; // state
    wpn.zoom = r.r_u8()?;
    // an update the editor's classes wrote ends here
    if r.r_elapsed() >= 4 || r.is_ini() && !r.r_eof() {
        r.r_u32()?;
    }
    Ok(())
}

fn w_magazined(o: &Object, w: &mut NetWriter) {
    w_weapon(o, w);
    w.w_u8(o.weapon().map_or(0, |wpn| wpn.cur_fire_mode));
}

fn r_magazined(o: &mut Object, h: &Header, r: &mut NetReader<'_>) -> PacketResult<()> {
    r_weapon(o, h, r)?;
    // with a trailing condition type absent the fire mode was taken for it: see `r_weapon`
    o.weapon_mut().cur_fire_mode = r.r_u8()?;
    Ok(())
}

fn w_shotgun(o: &Object, w: &mut NetWriter) {
    w_magazined(o, w);
    let ids = o.weapon().map_or(&[][..], |wpn| &wpn.ammo_ids[..]);
    w.w_u8(ids.len() as u8);
    for id in ids {
        w.w_u8(*id);
    }
}

fn r_shotgun(o: &mut Object, h: &Header, r: &mut NetReader<'_>) -> PacketResult<()> {
    r_magazined(o, h, r)?;
    let count = usize::from(r.r_u8()?);
    let wpn = o.weapon_mut();
    wpn.ammo_ids.clear();
    for i in 0..count {
        let id = r.r_u8()?;
        // the game drops cartridge ids beyond the magazine
        if i < wpn.mag_size as usize {
            wpn.ammo_ids.push(id);
        }
    }
    Ok(())
}

/// `CSE_ALifeCreatureActor::UPDATE_Write`.
fn w_actor(o: &Object, h: &Header, w: &mut NetWriter) {
    w_creature(o, h, w);
    let Some(a) = o.actor() else { return };
    w.w_u16(a.mstate);
    w.w_sdir(a.accel);
    w.w_sdir(a.velocity);
    w.w_float(a.radiation);
    w.w_u8(a.weapon);
    w.w_u16(a.num_items);
    if a.num_items == 0 {
        return;
    }
    if a.num_items == 1 {
        let default = AliveState::default();
        let s = a.alive.as_deref().unwrap_or(&default);
        w.w_u8(s.enabled);
        w.w_vec3(s.angular_vel);
        w.w_vec3(s.linear_vel);
        w.w_vec3(s.force);
        w.w_vec3(s.torque);
        w.w_vec3(s.position);
        for q in s.quaternion {
            w.w_float(q);
        }
        return;
    }
    w.w_u8(a.bone_data_size);
    w.w(&a.dead_body);
}

fn r_actor(o: &mut Object, r: &mut NetReader<'_>) -> PacketResult<()> {
    r_creature(o, r)?;
    let a = o.actor_mut();
    a.mstate = r.r_u16()?;
    a.accel = r.r_sdir()?;
    a.velocity = r.r_sdir()?;
    a.radiation = r.r_float()?;
    a.weapon = r.r_u8()?;
    a.num_items = r.r_u16()?;
    a.alive = None;
    a.dead_body.clear();
    if a.num_items == 0 {
        return Ok(());
    }
    if a.num_items == 1 {
        a.alive = Some(Box::new(AliveState {
            enabled: r.r_u8()?,
            angular_vel: r.r_vec3()?,
            linear_vel: r.r_vec3()?,
            force: r.r_vec3()?,
            torque: r.r_vec3()?,
            position: r.r_vec3()?,
            quaternion: [r.r_float()?, r.r_float()?, r.r_float()?, r.r_float()?],
        }));
        return Ok(());
    }
    a.bone_data_size = r.r_u8()?;
    let size = 24 + usize::from(a.bone_data_size) * usize::from(a.num_items);
    a.dead_body = r.take(size)?.to_vec();
    Ok(())
}

/// `UPDATE_Write` of the entity's class.
pub(crate) fn update_write(o: &Object, h: &Header, class: Class, w: &mut NetWriter) {
    match class {
        Class::ObjectPhysic => w_sync(o.physic().and_then(|p| p.sync.as_deref()), w),
        Class::StationaryMgun => {
            let (working, dir) = o.mgun().map_or((false, Vec3::ZERO), |m| (m.working, m.dest_enemy_dir));
            w.w_u8(u8::from(working));
            // `save_data(Fvector)`: raw
            let mut raw = [0u8; 12];
            for (slot, c) in raw.as_chunks_mut::<4>().0.iter_mut().zip(dir.to_array()) {
                slot.copy_from_slice(&c.to_le_bytes());
            }
            w.w(&raw);
        }
        Class::Item
        | Class::ItemDetector
        | Class::ItemArtefact
        | Class::ItemPda
        | Class::ItemDocument
        | Class::ItemGrenade
        | Class::ItemExplosive
        | Class::ItemBolt => w_item(o, w),
        Class::ItemTorch => {
            w_item(o, w);
            let flags = o.torch().map_or(0, |t| {
                u8::from(t.active) | u8::from(t.nightvision_active) << 1 | u8::from(t.attached) << 2
            });
            w.w_u8(flags);
        }
        Class::ItemAmmo => {
            w_item(o, w);
            w.w_u16(o.ammo().map_or(0, |a| a.elapsed));
        }
        Class::ItemWeapon => w_weapon(o, w),
        Class::ItemWeaponMagazined => w_magazined(o, w),
        Class::ItemWeaponMagazinedWGL => {
            w.w_u8(u8::from(o.weapon().is_some_and(|wpn| wpn.grenade_mode)));
            w_magazined(o, w);
        }
        Class::ItemWeaponShotGun | Class::ItemWeaponAutoShotGun => w_shotgun(o, w),
        Class::ItemCustomOutfit | Class::ItemHelmet => {
            w_item(o, w);
            w.w_float_q8(o.item().map_or(1.0, |i| i.condition), 0.0, 1.0);
        }
        Class::CreatureCrow | Class::CreaturePhantom => w_creature(o, h, w),
        Class::CreatureActor => w_actor(o, h, w),
        Class::MonsterZombie | Class::MonsterBase | Class::PsyDogPhantom => w_monster(o, h, w),
        Class::MonsterRat => {
            w_monster(o, h, w);
            w_item(o, w);
        }
        Class::MonsterBaseGroup => {
            w_monster(o, h, w);
            w.w_u32(u32::from(o.group().is_none_or(|g| g.create_spawn_positions)));
        }
        Class::HumanStalker => {
            w_monster(o, h, w);
            w.w_stringz(o.human().map_or("", |hm| hm.start_dialog.as_str()));
        }
        Class::Spectator
        | Class::Temporary
        | Class::GraphPoint
        | Class::AbstractVisual
        | Class::DynamicObjectVisual
        | Class::PhSkeletonObject
        | Class::SpaceRestrictor
        | Class::LevelChanger
        | Class::SmartZone
        | Class::HangingLamp
        | Class::Projector
        | Class::Helicopter
        | Class::Car
        | Class::Breakable
        | Class::Climable
        | Class::MountedWeapon
        | Class::TeamBaseZone
        | Class::InventoryBox
        | Class::Trader
        | Class::CustomZone
        | Class::AnomalousZone
        | Class::TorridZone
        | Class::ZoneVisual
        | Class::OnlineOfflineGroup
        | Class::SmartCover => {}
    }
}

/// `UPDATE_Read` of the entity's class: the update-only fields are taken, the rest is passed
/// over.
pub(crate) fn update_read(
    o: &mut Object,
    h: &Header,
    info: &ClassInfo,
    r: &mut NetReader<'_>,
) -> PacketResult<()> {
    match info.class {
        Class::ObjectPhysic => {
            // "backward compatibility": an update may be empty
            if !r.r_eof() {
                let sync = r_sync(r, false)?;
                o.physic_mut().sync = sync;
            }
        }
        Class::StationaryMgun => {
            let working = r.r_u8()? != 0;
            let raw = r.take(12)?;
            let mut dir = [0.0f32; 3];
            for (c, bytes) in dir.iter_mut().zip(raw.as_chunks::<4>().0) {
                let mut b = [0u8; 4];
                b.copy_from_slice(bytes);
                *c = f32::from_le_bytes(b);
            }
            let m = o.mgun_mut();
            m.working = working;
            m.dest_enemy_dir = Vec3::from_array(dir);
        }
        Class::Item
        | Class::ItemDetector
        | Class::ItemArtefact
        | Class::ItemPda
        | Class::ItemDocument
        | Class::ItemGrenade
        | Class::ItemExplosive
        | Class::ItemBolt => r_item(o, h, r)?,
        Class::ItemTorch => {
            r_item(o, h, r)?;
            let flags = r.r_u8()?;
            let t = o.torch_mut();
            t.active = flags & 1 != 0;
            t.nightvision_active = flags & 2 != 0;
            t.attached = flags & 4 != 0;
        }
        Class::ItemAmmo => {
            r_item(o, h, r)?;
            r.r_u16()?;
        }
        Class::ItemWeapon => r_weapon(o, h, r)?,
        Class::ItemWeaponMagazined => r_magazined(o, h, r)?,
        Class::ItemWeaponMagazinedWGL => {
            o.weapon_mut().grenade_mode = r.r_u8()? != 0;
            r_magazined(o, h, r)?;
        }
        Class::ItemWeaponShotGun | Class::ItemWeaponAutoShotGun => r_shotgun(o, h, r)?,
        Class::ItemCustomOutfit | Class::ItemHelmet => {
            r_item(o, h, r)?;
            r.r_u8()?;
        }
        Class::CreatureCrow | Class::CreaturePhantom => r_creature(o, r)?,
        Class::CreatureActor => r_actor(o, r)?,
        Class::MonsterZombie | Class::MonsterBase | Class::PsyDogPhantom => r_monster(o, r)?,
        Class::MonsterRat => {
            r_monster(o, r)?;
            r_item(o, h, r)?;
        }
        Class::MonsterBaseGroup => {
            r_monster(o, r)?;
            o.group_mut().create_spawn_positions = r.r_u32()? != 0;
        }
        Class::HumanStalker => {
            r_monster(o, r)?;
            if h.version < 110 {
                r.r_u32()?;
                r.r_u32()?;
                r.r_u32()?;
            }
            o.human_mut().start_dialog = r.r_stringz()?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::TextEncoding;

    #[test]
    fn an_idle_item_sends_a_single_zero() {
        let mut w = NetWriter::new(TextEncoding::Cp1251);
        w_sync(None, &mut w);
        assert_eq!(w.bytes(), [0]);
        assert_eq!(r_sync(&mut NetReader::new(&[0]), false).unwrap(), None);
    }

    #[test]
    fn the_sync_block_packs_count_and_mask_into_one_byte() {
        let s = SyncState {
            num_items: 3,
            enabled: true,
            position: Vec3::new(1.0, 2.0, 3.0),
            linear_vel: Vec3::new(0.0, 0.5, 0.0),
            ..SyncState::default()
        };
        let mut w = NetWriter::new(TextEncoding::Cp1251);
        w_sync(Some(&s), &mut w);
        // enabled and "angular is zero": mask 0b011 above the count
        assert_eq!(w.bytes()[0], 3 | 0b011 << 5);
        // force, torque, position, quaternion, linear velocity, the freeze byte
        assert_eq!(w.bytes().len(), 1 + 12 * 3 + 16 + 12 + 1);
        let back = r_sync(&mut NetReader::new(w.bytes()), false).unwrap().unwrap();
        assert_eq!(*back, s);
    }

    #[test]
    fn an_old_item_update_is_quantised() {
        // the game's branch for versions below 122: no force and torque, 8-bit values
        let mut w = NetWriter::new(TextEncoding::Cp1251);
        w.w_u8(1 | (SYNC_ANGULAR_NULL | SYNC_LINEAR_NULL) << 5);
        w.w_vec3(Vec3::new(4.0, 5.0, 6.0));
        for q in [0u8, 0, 255, 0] {
            w.w_u8(q);
        }
        let s = r_sync(&mut NetReader::new(w.bytes()), true).unwrap().unwrap();
        assert_eq!(s.position, Vec3::new(4.0, 5.0, 6.0));
        assert!(s.quaternion[2] > 0.999 && s.quaternion[0] == 0.0);
        assert_eq!((s.force, s.angular_vel), (Vec3::ZERO, Vec3::ZERO));
    }
}
