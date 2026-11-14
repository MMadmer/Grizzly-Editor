//! The linked game's world as its `all.spawn` holds it (the file the engine loads): the levels
//! of the game graph, every level changer with its level and where it leads, the patrol names.
//! Read only - nothing here writes a file of the game.

use std::sync::Arc;

use gz_chunk::Reader;
use gz_entities::{Entity, Settings};
use gz_fmt_spawn::{SpawnFile, SpawnIndex, peek_packet};
use gz_ltx::Ltx;
use rayon::prelude::*;

/// A level changer of the linked game.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameTransition {
    /// The object name: what `[world] locked` and `world.transition` name.
    pub name: String,
    /// Where it stands.
    pub level: String,
    /// Where it leads.
    pub to_level: String,
    pub position: [f32; 3],
}

/// What was read out of the linked game.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GameWorld {
    /// The file read; the lists are empty otherwise and `error` says why.
    pub ok: bool,
    pub error: String,
    /// Sorted by name.
    pub levels: Vec<String>,
    /// Sorted by level, then by name.
    pub transitions: Vec<GameTransition>,
    /// The game's own patrol paths, sorted, each once: a module path of the same name loses to
    /// them.
    pub patrols: Vec<String>,
}

impl GameWorld {
    #[must_use]
    pub fn failed(error: impl Into<String>) -> Self {
        Self { error: error.into(), ..Self::default() }
    }

    /// The level exists, any case.
    #[must_use]
    pub fn level_exists(&self, name: &str) -> bool {
        self.levels.iter().any(|l| l.eq_ignore_ascii_case(name))
    }

    /// A transition by its object name.
    #[must_use]
    pub fn transition(&self, name: &str) -> Option<&GameTransition> {
        self.transitions.iter().find(|t| t.name == name)
    }

    /// The game has a patrol path of this name.
    #[must_use]
    pub fn patrol_exists(&self, name: &str) -> bool {
        self.patrols.binary_search_by(|p| p.as_str().cmp(name)).is_ok()
    }

    /// How many transitions stand on a level.
    #[must_use]
    pub fn transitions_on(&self, level: &str) -> usize {
        self.transitions.iter().filter(|t| t.level == level).count()
    }
}

/// The linked game's configs as `gz-entities` asks them.
struct LtxSettings<'a>(&'a Ltx);

impl Settings for LtxSettings<'_> {
    fn section_exist(&self, section: &str) -> bool {
        self.0.section_exist(&section.to_ascii_lowercase())
    }

    fn r_string(&self, section: &str, key: &str) -> Option<&str> {
        let s = section.to_ascii_lowercase();
        if !self.0.line_exist(&s, key) {
            return None;
        }
        Some(self.0.r_string_raw(&s, key).ok().flatten().unwrap_or(""))
    }
}

/// A section the linked game spawns as a level changer: class `LVL_CHNG`, or - without the
/// configs - a name that says so.
pub(crate) fn is_level_changer(section: &str, ltx: Option<&Ltx>) -> bool {
    let s = section.to_ascii_lowercase();
    match ltx {
        Some(l) if l.section_exist(&s) && l.line_exist(&s, "class") => {
            l.r_string(&s, "class").is_ok_and(|c| c.trim().eq_ignore_ascii_case("LVL_CHNG"))
        }
        _ => section.starts_with("level_changer"),
    }
}

/// Where a level changer leads, walked the way the engine reads it: the
/// header of a spawn packet, then the state chain object -> shape -> restrictor
/// -> changer. What a packet the entity table cannot decode still says.
pub(crate) fn walk_destination(packet: &[u8]) -> Option<String> {
    let mut s = Reader::new(packet);
    if s.r_u16().ok()? != 1 {
        return None;
    }
    s.r_stringz_raw().ok()?;
    s.r_stringz_raw().ok()?;
    // game id, respawn point; position, angle; respawn time, id, parent, phantom
    s.advance(2 + 12 + 12 + 8).ok()?;
    let flags = s.r_u16().ok()?;
    let version = if flags & (1 << 5) != 0 { s.r_u16().ok()? } else { 0 };
    // older formats predate every game this editor links to
    if version < 112 {
        return None;
    }
    if version > 120 {
        s.advance(2).ok()?;
    }
    s.advance(2).ok()?;
    let client_data = usize::from(s.r_u16().ok()?);
    // client data, spawn id, state size
    s.advance(client_data + 2 + 2).ok()?;
    // CSE_ALifeObject: graph id, distance, direct control, node, flags, custom data, story ids
    s.advance(2 + 16).ok()?;
    s.r_stringz_raw().ok()?;
    s.advance(8).ok()?;
    // CSE_Shape: a sphere is 16 bytes, a box four vectors
    let shapes = s.r_u8().ok()?;
    for _ in 0..shapes {
        let kind = s.r_u8().ok()?;
        s.advance(if kind == 0 { 16 } else { 48 }).ok()?;
    }
    // restrictor type; next vertex, next node, next position, angles
    s.advance(1 + 2 + 4 + 12 + 12).ok()?;
    Some(gz_base::text::cp1251_to_utf8(s.r_stringz_raw().ok()?).into_owned())
}

/// Where a level changer leads: the entity's own reading, the walk when the entity table
/// keeps it opaque.
fn destination(packet: &[u8], settings: Option<&LtxSettings<'_>>) -> Option<String> {
    if let Some(st) = settings
        && let Ok(e) = Entity::from_spawn_packet(packet, st)
        && let Some(d) = e.level_changer()
    {
        return Some(d.level.to_string());
    }
    walk_destination(packet)
}

/// Reads the linked game's `all.spawn`. `configs` tells level changers by their class (by their
/// name without them). The index and the level changers are read in parallel on the caller's
/// pool. Bounds are checked everywhere: a file the editor cannot make sense of leaves the
/// lists empty with the reason, it never takes the editor down.
#[must_use]
pub fn read_all_spawn(bytes: Arc<[u8]>, configs: Option<&Ltx>) -> GameWorld {
    let top = Reader::new(&bytes);
    if top.find_chunk(4).is_none() {
        return GameWorld::failed("all.spawn has no game graph (chunk 4)");
    }
    let file = match SpawnFile::parse(bytes.clone()) {
        Ok(f) => f,
        Err(e) => return GameWorld::failed(format!("the game graph of all.spawn could not be read: {e}")),
    };
    if file.game_graph.is_none() {
        return GameWorld::failed("the game graph of all.spawn could not be read");
    }
    let index = SpawnIndex::build(&file);
    let mut levels: Vec<String> = index.levels.iter().map(|l| l.name.to_string()).collect();
    levels.sort();
    levels.dedup();
    let mut patrols: Vec<String> =
        index.patrol_names.iter().filter(|n| !n.is_empty()).map(ToString::to_string).collect();
    patrols.sort();
    patrols.dedup();
    if Reader::new(&bytes).find_chunk(1).is_none() {
        return GameWorld {
            error: "all.spawn has no objects (chunk 1)".into(),
            levels,
            patrols,
            ..GameWorld::default()
        };
    }

    let settings = configs.map(LtxSettings);
    let mut transitions: Vec<GameTransition> = index
        .entities
        .par_iter()
        .zip(file.vertices.par_iter())
        .filter(|(e, _)| !e.damaged && is_level_changer(&e.section, configs))
        .filter_map(|(e, v)| {
            let head = peek_packet(&v.spawn).ok()?;
            if head.version < 112 {
                return None;
            }
            let to_level = destination(&v.spawn, settings.as_ref())?;
            let level = e.level.and_then(|id| index.level_name(id)).unwrap_or("").to_owned();
            Some(GameTransition {
                name: e.name.to_string(),
                level,
                to_level,
                position: e.position.to_array(),
            })
        })
        .collect();
    transitions.sort_by(|a, b| a.level.cmp(&b.level).then_with(|| a.name.cmp(&b.name)));
    GameWorld { ok: true, error: String::new(), levels, transitions, patrols }
}
