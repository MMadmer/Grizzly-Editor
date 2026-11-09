//! What Dead Air's script classes write after the engine's state.
//!
//! The game's `se_*.script` classes override `STATE_Write` / `STATE_Read`: they call the
//! engine class and then put fields of their own into the same block. The layouts here are
//! those of `se_stalker`, `se_monster`, `se_actor`, `se_level_changer`, `se_heli`,
//! `se_smart_cover`, `se_artefact`, `se_item.se_eatable`, `smart_terrain` and
//! `sim_squad_scripted`.
//!
//! A block that ends before the script fields is taken as it is: [`Script::None`] is written
//! back as nothing. A new entity gets the fields the scripts write for an object that has
//! never been in the game.

use smol_str::SmolStr;

use crate::class::ScriptKind;
use crate::packet::{NetReader, NetWriter, PacketResult};

const NIL: SmolStr = SmolStr::new_static("nil");

/// `se_heli`: written only for a helicopter that has a respawn point.
#[derive(Clone, Debug, PartialEq)]
pub struct HeliExt {
    pub assigned_target_id: u16,
    pub current_target_id: u16,
    pub smart_terrain_id: u16,
    pub respawn_point_id: u16,
    pub respawn_point_prop_section: SmolStr,
}

/// `se_artefact`: the condition and the sixteen generated properties, in packet order
/// (weight, the five restore speeds, the additional weight, the nine immunities).
#[derive(Clone, Debug, PartialEq)]
pub struct ArtefactExt {
    pub offline_condition: f32,
    pub properties: [f32; 16],
    /// False asks the game to generate the properties when it first saves the artefact.
    pub generate: bool,
}

impl Default for ArtefactExt {
    fn default() -> Self {
        // the script's own fallback for an artefact without a saved condition
        Self { offline_condition: 0.999_999_9, properties: [0.0; 16], generate: false }
    }
}

/// One job row of a smart terrain.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainJob {
    pub npc_id: u16,
    pub job_section: SmolStr,
    pub begin_job: bool,
    pub need_job: SmolStr,
}

/// `utils.w_CTime`: year - 2000, month, day, hour, minute, second, milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScriptTime {
    pub date: [u8; 6],
    pub ms: u16,
}

/// The respawn state of a smart terrain.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct TerrainRespawn {
    /// Respawn sections and how many of each were spawned.
    pub spawned: Vec<(SmolStr, u8)>,
    pub last_update: Option<ScriptTime>,
}

/// `se_smart_terrain`.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainExt {
    pub arriving_npc: Vec<u16>,
    pub jobs: Vec<TerrainJob>,
    /// When the NPC of each job section died (`utils.w_CTime`: `None` is no time).
    pub dead_time: Vec<(SmolStr, Option<ScriptTime>)>,
    pub respawn: Option<TerrainRespawn>,
    pub faction: SmolStr,
}

impl Default for TerrainExt {
    fn default() -> Self {
        Self { arriving_npc: Vec::new(), jobs: Vec::new(), dead_time: Vec::new(), respawn: None, faction: NIL }
    }
}

/// `sim_squad_scripted`: every field is the `tostring` of a number, a string or `nil`.
#[derive(Clone, Debug, PartialEq)]
pub struct SquadExt {
    pub current_target_id: SmolStr,
    pub assigned_target_id: SmolStr,
    pub respawn_point_id: SmolStr,
    pub respawn_point_prop_section: SmolStr,
    pub smart_id: SmolStr,
    pub online: SmolStr,
}

impl Default for SquadExt {
    fn default() -> Self {
        Self {
            current_target_id: NIL,
            assigned_target_id: NIL,
            respawn_point_id: NIL,
            respawn_point_prop_section: NIL,
            smart_id: NIL,
            online: SmolStr::new_static("false"),
        }
    }
}

/// The script fields of an entity.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Script {
    /// The class writes none, or the block ended before them.
    #[default]
    None,
    /// `se_stalker`: the level vertex and the logic section, as `tostring` made them.
    Stalker { level_vertex_id: SmolStr, active_section: SmolStr },
    /// `se_monster`: the same two.
    Monster { level_vertex_id: SmolStr, active_section: SmolStr },
    /// `se_actor`.
    Actor { start_position_filled: bool },
    /// `se_level_changer`.
    LevelChanger { enabled: bool, hint: SmolStr },
    /// `se_heli`.
    Heli(Option<HeliExt>),
    /// `se_smart_cover`: the description the loopholes belong to, and each loophole's switch.
    SmartCover { last_description: SmolStr, loopholes: Vec<(SmolStr, bool)> },
    /// `se_artefact`.
    Artefact(Box<ArtefactExt>),
    /// `se_eatable`: the uses left.
    Eatable { remaining_uses: u8 },
    /// `se_smart_terrain`.
    SmartTerrain(Box<TerrainExt>),
    /// `sim_squad_scripted`.
    Squad(Box<SquadExt>),
}

impl Script {
    /// What the script class writes for an entity that was just made.
    pub(crate) fn new(kind: ScriptKind) -> Self {
        match kind {
            ScriptKind::None => Self::None,
            ScriptKind::Stalker => Self::Stalker { level_vertex_id: NIL, active_section: NIL },
            ScriptKind::Monster => Self::Monster { level_vertex_id: NIL, active_section: NIL },
            ScriptKind::Actor => Self::Actor { start_position_filled: false },
            ScriptKind::LevelChanger => Self::LevelChanger { enabled: false, hint: NIL },
            ScriptKind::Heli => Self::Heli(None),
            ScriptKind::SmartCover => Self::SmartCover { last_description: NIL, loopholes: Vec::new() },
            ScriptKind::Artefact => Self::Artefact(Box::default()),
            ScriptKind::Eatable => Self::Eatable { remaining_uses: 0 },
            ScriptKind::SmartTerrain => Self::SmartTerrain(Box::default()),
            ScriptKind::Squad => Self::Squad(Box::default()),
        }
    }

    /// The script's `STATE_Read`, after the engine class has read its part.
    /// `description` is the smart cover's own, for packets older than its script fields.
    pub(crate) fn read(
        kind: ScriptKind,
        r: &mut NetReader<'_>,
        script_version: u16,
        description: &str,
    ) -> PacketResult<Self> {
        if kind == ScriptKind::None {
            return Ok(Self::None);
        }
        let empty = r.r_eof();
        Ok(match kind {
            ScriptKind::None => Self::None,
            ScriptKind::Stalker => {
                if empty {
                    return Ok(Self::None);
                }
                let (level_vertex_id, active_section) =
                    if script_version > 10 { (r.r_stringz()?, r.r_stringz()?) } else { (NIL, NIL) };
                // `death_droped`: read and dropped; the script always writes false
                r.r_u8()?;
                Self::Stalker { level_vertex_id, active_section }
            }
            ScriptKind::Monster => {
                if script_version > 10 {
                    if empty {
                        return Ok(Self::None);
                    }
                    Self::Monster { level_vertex_id: r.r_stringz()?, active_section: r.r_stringz()? }
                } else {
                    Self::Monster { level_vertex_id: NIL, active_section: NIL }
                }
            }
            ScriptKind::Actor => {
                if empty {
                    return Ok(Self::None);
                }
                let start_position_filled = r.r_u8()? != 0;
                r.r_u16()?; // save marker
                Self::Actor { start_position_filled }
            }
            ScriptKind::LevelChanger => {
                if empty {
                    return Ok(Self::None);
                }
                let enabled = r.r_u8()? != 0;
                let hint = r.r_stringz()?;
                r.r_u16()?; // save marker
                Self::LevelChanger { enabled, hint }
            }
            ScriptKind::Heli => {
                if empty {
                    return Ok(Self::Heli(None));
                }
                Self::Heli(Some(HeliExt {
                    assigned_target_id: r.r_u16()?,
                    current_target_id: r.r_u16()?,
                    smart_terrain_id: r.r_u16()?,
                    respawn_point_id: r.r_u16()?,
                    respawn_point_prop_section: r.r_stringz()?,
                }))
            }
            ScriptKind::SmartCover => {
                if script_version >= 9 {
                    if empty {
                        return Ok(Self::None);
                    }
                    let last_description = r.r_stringz()?;
                    let n = usize::from(r.r_u8()?);
                    let mut loopholes = Vec::with_capacity(r.checked_count(n, 2)?);
                    for _ in 0..n {
                        loopholes.push((r.r_stringz()?, r.r_u8()? != 0));
                    }
                    Self::SmartCover { last_description, loopholes }
                } else {
                    let last = if description.is_empty() { NIL } else { SmolStr::new(description) };
                    Self::SmartCover { last_description: last, loopholes: Vec::new() }
                }
            }
            ScriptKind::Artefact => {
                if empty {
                    return Ok(Self::None);
                }
                let offline_condition = r.r_float()?;
                let mut properties = [0.0f32; 16];
                for p in &mut properties {
                    *p = r.r_float()?;
                }
                let generate = r.r_u8()? != 0;
                Self::Artefact(Box::new(ArtefactExt { offline_condition, properties, generate }))
            }
            ScriptKind::Eatable => {
                if empty {
                    return Ok(Self::None);
                }
                Self::Eatable { remaining_uses: r.r_u8()? }
            }
            ScriptKind::SmartTerrain => {
                if empty {
                    return Ok(Self::None);
                }
                let n = usize::from(r.r_u8()?);
                let mut arriving_npc = Vec::with_capacity(r.checked_count(n, 2)?);
                for _ in 0..n {
                    arriving_npc.push(r.r_u16()?);
                }
                let n = usize::from(r.r_u8()?);
                let mut jobs = Vec::with_capacity(r.checked_count(n, 5)?);
                for _ in 0..n {
                    jobs.push(TerrainJob {
                        npc_id: r.r_u16()?,
                        job_section: r.r_stringz()?,
                        begin_job: r.r_u8()? != 0,
                        need_job: r.r_stringz()?,
                    });
                }
                let n = usize::from(r.r_u8()?);
                let mut dead_time = Vec::with_capacity(r.checked_count(n, 2)?);
                for _ in 0..n {
                    dead_time.push((r.r_stringz()?, r_ctime(r)?));
                }
                let respawn = if r.r_u8()? != 0 {
                    let n = usize::from(r.r_u8()?);
                    let mut spawned = Vec::with_capacity(r.checked_count(n, 2)?);
                    for _ in 0..n {
                        spawned.push((r.r_stringz()?, r.r_u8()?));
                    }
                    let last_update = r_ctime(r)?;
                    Some(TerrainRespawn { spawned, last_update })
                } else {
                    None
                };
                Self::SmartTerrain(Box::new(TerrainExt { arriving_npc, jobs, dead_time, respawn, faction: r.r_stringz()? }))
            }
            ScriptKind::Squad => {
                if empty {
                    return Ok(Self::None);
                }
                let ext = SquadExt {
                    current_target_id: r.r_stringz()?,
                    assigned_target_id: r.r_stringz()?,
                    respawn_point_id: r.r_stringz()?,
                    respawn_point_prop_section: r.r_stringz()?,
                    smart_id: r.r_stringz()?,
                    online: r.r_stringz()?,
                };
                r.r_u16()?; // save marker
                Self::Squad(Box::new(ext))
            }
        })
    }

    /// The script's `STATE_Write`, after the engine class has written its part.
    pub(crate) fn write(&self, w: &mut NetWriter) {
        match self {
            Self::None | Self::Heli(None) => {}
            Self::Stalker { level_vertex_id, active_section } => {
                w.w_stringz(level_vertex_id);
                w.w_stringz(active_section);
                w.w_u8(0);
            }
            Self::Monster { level_vertex_id, active_section } => {
                w.w_stringz(level_vertex_id);
                w.w_stringz(active_section);
            }
            Self::Actor { start_position_filled } => {
                let mark = w.w_tell();
                w.w_u8(u8::from(*start_position_filled));
                save_marker(w, mark);
            }
            Self::LevelChanger { enabled, hint } => {
                let mark = w.w_tell();
                w.w_u8(u8::from(*enabled));
                w.w_stringz(hint);
                save_marker(w, mark);
            }
            Self::Heli(Some(h)) => {
                // `if (self.respawn_point_prop_section)`: "nil" was read back as no section
                if h.respawn_point_prop_section != "nil" {
                    w.w_u16(h.assigned_target_id);
                    w.w_u16(h.current_target_id);
                    w.w_u16(h.smart_terrain_id);
                    w.w_u16(h.respawn_point_id);
                    w.w_stringz(&h.respawn_point_prop_section);
                }
            }
            Self::SmartCover { last_description, loopholes } => {
                w.w_stringz(last_description);
                w.w_u8(loopholes.len() as u8);
                for (id, on) in loopholes {
                    w.w_stringz(id);
                    w.w_u8(u8::from(*on));
                }
            }
            Self::Artefact(a) => {
                w.w_float(a.offline_condition);
                for p in a.properties {
                    w.w_float(p);
                }
                w.w_u8(u8::from(a.generate));
            }
            Self::Eatable { remaining_uses } => w.w_u8(*remaining_uses),
            Self::SmartTerrain(t) => {
                w.w_u8(t.arriving_npc.len() as u8);
                for id in &t.arriving_npc {
                    w.w_u16(*id);
                }
                w.w_u8(t.jobs.len() as u8);
                for job in &t.jobs {
                    w.w_u16(job.npc_id);
                    w.w_stringz(&job.job_section);
                    w.w_u8(u8::from(job.begin_job));
                    w.w_stringz(&job.need_job);
                }
                w.w_u8(t.dead_time.len() as u8);
                for (section, time) in &t.dead_time {
                    w.w_stringz(section);
                    w_ctime(w, *time);
                }
                match &t.respawn {
                    Some(respawn) => {
                        w.w_u8(1);
                        w.w_u8(respawn.spawned.len() as u8);
                        for (section, num) in &respawn.spawned {
                            w.w_stringz(section);
                            w.w_u8(*num);
                        }
                        w_ctime(w, respawn.last_update);
                    }
                    None => w.w_u8(0),
                }
                w.w_stringz(&t.faction);
            }
            Self::Squad(s) => {
                let mark = w.w_tell();
                w.w_stringz(&s.current_target_id);
                w.w_stringz(&s.assigned_target_id);
                w.w_stringz(&s.respawn_point_id);
                w.w_stringz(&s.respawn_point_prop_section);
                w.w_stringz(&s.smart_id);
                w.w_stringz(&s.online);
                save_marker(w, mark);
            }
        }
    }

    /// `se_smart_cover:FillProps`: when the description changes the loopholes start over, and
    /// every loophole of the description that is not listed yet is switched on.
    pub(crate) fn sync_loopholes(&mut self, description: &str, ids: &[&str]) {
        let Self::SmartCover { last_description, loopholes } = self else { return };
        if last_description != description {
            loopholes.clear();
            *last_description = SmolStr::new(description);
        }
        for id in ids {
            if !loopholes.iter().any(|(have, _)| have == id) {
                loopholes.push((SmolStr::new(id), true));
            }
        }
    }
}

/// `utils.r_CTime`: a flag, then year - 2000, month, day, hour, minute, second, milliseconds.
fn r_ctime(r: &mut NetReader<'_>) -> PacketResult<Option<ScriptTime>> {
    if r.r_u8()? == 0 {
        return Ok(None);
    }
    let mut date = [0u8; 6];
    for d in &mut date {
        *d = r.r_u8()?;
    }
    Ok(Some(ScriptTime { date, ms: r.r_u16()? }))
}

/// `utils.w_CTime`.
fn w_ctime(w: &mut NetWriter, time: Option<ScriptTime>) {
    match time {
        Some(t) => {
            w.w_u8(1);
            for d in t.date {
                w.w_u8(d);
            }
            w.w_u16(t.ms);
        }
        None => w.w_u8(0),
    }
}

/// `set_save_marker(packet, "save", true, ..)`: the length of what was written since `mark`.
fn save_marker(w: &mut NetWriter, mark: usize) {
    let dif = w.w_tell().saturating_sub(mark);
    w.w_u16(dif as u16);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packet::TextEncoding;

    fn bytes(s: &Script) -> Vec<u8> {
        let mut w = NetWriter::new(TextEncoding::Cp1251);
        s.write(&mut w);
        w.into_bytes()
    }

    fn back(kind: ScriptKind, data: &[u8], script_version: u16) -> Script {
        Script::read(kind, &mut NetReader::new(data), script_version, "").unwrap()
    }

    #[test]
    fn a_new_stalker_writes_what_the_script_writes() {
        let s = Script::new(ScriptKind::Stalker);
        assert_eq!(bytes(&s), b"nil\0nil\0\0");
        assert_eq!(back(ScriptKind::Stalker, &bytes(&s), 12), s);
    }

    #[test]
    fn markers_hold_the_length_between_them() {
        let actor = Script::new(ScriptKind::Actor);
        assert_eq!(bytes(&actor), [0, 1, 0]);
        let changer = Script::LevelChanger { enabled: true, hint: SmolStr::new("to_bar") };
        assert_eq!(bytes(&changer), b"\x01to_bar\0\x08\0");
        let squad = Script::new(ScriptKind::Squad);
        let b = bytes(&squad);
        assert_eq!(&b[..b.len() - 2], b"nil\0nil\0nil\0nil\0nil\0false\0");
        assert_eq!(&b[b.len() - 2..], [26, 0]);
        for (kind, s) in [(ScriptKind::Actor, actor), (ScriptKind::LevelChanger, changer), (ScriptKind::Squad, squad)] {
            assert_eq!(back(kind, &bytes(&s), 12), s);
        }
    }

    #[test]
    fn every_kind_reads_back_what_it_wrote() {
        let terrain = Script::SmartTerrain(Box::new(TerrainExt {
            arriving_npc: vec![5, 9],
            jobs: vec![TerrainJob { npc_id: 7, job_section: "walker".into(), begin_job: true, need_job: "nil".into() }],
            dead_time: vec![("guard".into(), Some(ScriptTime { date: [12, 5, 1, 9, 0, 0], ms: 0 })), ("camper".into(), None)],
            respawn: Some(TerrainRespawn {
                spawned: vec![("stalker_sim".into(), 2)],
                last_update: Some(ScriptTime { date: [12, 5, 1, 8, 30, 0], ms: 250 }),
            }),
            faction: "stalker".into(),
        }));
        let cover = Script::SmartCover {
            last_description: "combat_prone".into(),
            loopholes: vec![("prone".into(), true), ("stand".into(), false)],
        };
        let heli = Script::Heli(Some(HeliExt {
            assigned_target_id: 1,
            current_target_id: 65535,
            smart_terrain_id: 3,
            respawn_point_id: 4,
            respawn_point_prop_section: "heli_sim".into(),
        }));
        let artefact = Script::Artefact(Box::new(ArtefactExt {
            offline_condition: 0.5,
            properties: [1.0; 16],
            generate: true,
        }));
        for (kind, s) in [
            (ScriptKind::SmartTerrain, terrain),
            (ScriptKind::SmartCover, cover),
            (ScriptKind::Heli, heli),
            (ScriptKind::Artefact, artefact),
            (ScriptKind::Eatable, Script::Eatable { remaining_uses: 3 }),
            (ScriptKind::Monster, Script::Monster { level_vertex_id: "120".into(), active_section: "nil".into() }),
        ] {
            assert_eq!(back(kind, &bytes(&s), 12), s, "{kind:?}");
        }
        assert_eq!(bytes(&Script::new(ScriptKind::Artefact)).len(), 17 * 4 + 1);
        assert_eq!(bytes(&Script::new(ScriptKind::SmartTerrain)), b"\0\0\0\0nil\0");
    }

    #[test]
    fn a_block_without_script_fields_stays_without() {
        for kind in [
            ScriptKind::Stalker,
            ScriptKind::Monster,
            ScriptKind::Actor,
            ScriptKind::LevelChanger,
            ScriptKind::SmartCover,
            ScriptKind::Artefact,
            ScriptKind::Eatable,
            ScriptKind::SmartTerrain,
            ScriptKind::Squad,
        ] {
            let s = back(kind, &[], 12);
            assert_eq!(s, Script::None, "{kind:?}");
            assert!(bytes(&s).is_empty());
        }
        assert_eq!(back(ScriptKind::Heli, &[], 12), Script::Heli(None));
    }

    #[test]
    fn old_script_versions_have_fewer_fields() {
        // se_stalker before script version 11: the flag only
        assert_eq!(back(ScriptKind::Stalker, &[0], 10), Script::new(ScriptKind::Stalker));
        // se_monster before 11: nothing
        assert_eq!(back(ScriptKind::Monster, &[], 10), Script::new(ScriptKind::Monster));
        // se_smart_cover before 9: the loopholes come from the description
        let cover = Script::read(ScriptKind::SmartCover, &mut NetReader::new(&[]), 8, "combat").unwrap();
        assert_eq!(cover, Script::SmartCover { last_description: "combat".into(), loopholes: Vec::new() });
    }

    #[test]
    fn a_cut_block_is_an_error() {
        assert!(Script::read(ScriptKind::Squad, &mut NetReader::new(b"nil\0nil\0"), 12, "").is_err());
        assert!(Script::read(ScriptKind::SmartTerrain, &mut NetReader::new(&[200]), 12, "").is_err());
    }

    #[test]
    fn loopholes_follow_the_description() {
        let mut s = Script::new(ScriptKind::SmartCover);
        s.sync_loopholes("combat", &["a", "b"]);
        assert_eq!(
            s,
            Script::SmartCover { last_description: "combat".into(), loopholes: vec![("a".into(), true), ("b".into(), true)] }
        );
        if let Script::SmartCover { loopholes, .. } = &mut s {
            loopholes[0].1 = false;
        }
        s.sync_loopholes("combat", &["a", "b", "c"]);
        assert!(matches!(&s, Script::SmartCover { loopholes, .. } if loopholes.len() == 3 && !loopholes[0].1));
        s.sync_loopholes("other", &["x"]);
        assert!(matches!(&s, Script::SmartCover { loopholes, .. } if loopholes.len() == 1));
    }
}
