//! Every entity of the game's `all.spawn`, decoded with the game's configs: each reads, and
//! written anew from what was decoded - header, state and update - gives the bytes it was read
//! from. What does not decode is counted by its reason; a packet of today's layout must decode.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use gz_base::text::cp1251_to_utf8;
use gz_corpus::{Game, check_all, finish, key};
use gz_entities::{Entity, OpaqueReason, Settings};
use gz_fmt_spawn::SpawnFile;
use gz_ltx::{Dialect, Loader, Ltx, ParsedFile, parse_file};
use rayon::prelude::*;

struct Configs(BTreeMap<String, ParsedFile>);

impl Loader for Configs {
    fn file(&self, path: &str) -> Option<&ParsedFile> {
        self.0.get(&key(path))
    }

    fn list(&self, dir: &str, mask: &str) -> Vec<String> {
        let mut prefix = key(dir);
        prefix.push('/');
        let tail = mask.trim_start_matches('*').to_lowercase();
        self.0
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .filter(|rel| rel.ends_with(&tail))
            .map(|rel| rel.replace('/', "\\"))
            .collect()
    }
}

/// The game's `system.ltx` tree, as the game reads it.
fn system_ltx(game: &Game) -> Ltx {
    let files = game
        .files()
        .par_iter()
        .filter(|e| e.key.starts_with("configs/") && e.key.ends_with(".ltx"))
        .filter_map(|e| {
            Some((e.key.clone(), parse_file(&cp1251_to_utf8(&game.read(e).ok()?), Dialect::Game)))
        })
        .collect();
    Ltx::assemble("configs\\system.ltx", &Configs(files), Dialect::Game).map(|l| l.ltx).unwrap_or_default()
}

struct LtxSettings<'a>(&'a Ltx);

impl Settings for LtxSettings<'_> {
    fn section_exist(&self, section: &str) -> bool {
        self.0.section_exist(&section.to_ascii_lowercase())
    }

    fn r_string(&self, section: &str, key: &str) -> Option<&str> {
        let s = section.to_ascii_lowercase();
        self.0.r_string_raw(&s, key).ok().flatten()
    }
}

#[test]
fn every_entity_of_all_spawn_decodes_and_writes_back() {
    let Some(game) = gz_corpus::game() else { return };
    let started = Instant::now();
    let game = game.unwrap();
    let ltx = system_ltx(&game);
    assert!(ltx.section_count() > 1000);
    let settings = LtxSettings(&ltx);
    let file = SpawnFile::parse(Arc::from(game.read_path("spawns/all.spawn").unwrap())).unwrap();
    let reasons: BTreeMap<String, AtomicUsize> = [
        OpaqueReason::UnknownClass,
        OpaqueReason::NewerVersion,
        OpaqueReason::NoVersion,
        OpaqueReason::BadSection,
        OpaqueReason::BadState,
        OpaqueReason::Lossy,
    ]
    .into_iter()
    .map(|r| (format!("{r:?}"), AtomicUsize::new(0)))
    .collect();
    let short_packets = AtomicUsize::new(0);
    let normalised = AtomicUsize::new(0);
    let upgraded = AtomicUsize::new(0);
    let failed = check_all(
        &file.vertices,
        |v| format!("spawn {}", v.id),
        |v| {
            let e = Entity::from_packets(&v.spawn, &v.update, &settings).map_err(|e| e.to_string())?;
            if let Some(o) = e.opaque() {
                if let Some(n) = reasons.get(&format!("{:?}", o.reason)) {
                    n.fetch_add(1, Ordering::Relaxed);
                }
                // Dead Air changed some sections' classes after its all.spawn was made (repair kits
                // and containers became PDAs, loose ammunition an item): their packets end before
                // what the class reads - the game's reader cannot read them either - and stay
                // opaque. Any other packet of the layout this crate writes must decode.
                let short = o.reason == OpaqueReason::BadState && o.detail.contains("bytes early");
                if short {
                    short_packets.fetch_add(1, Ordering::Relaxed);
                } else if matches!(o.reason, OpaqueReason::BadState | OpaqueReason::Lossy) {
                    let class =
                        gz_entities::class_of_section(e.section(), &settings).map_or("?", |c| c.clsid);
                    return Err(format!(
                        "{class} [{}] '{}': {} {}",
                        e.section(),
                        e.name(),
                        o.reason.text(),
                        o.detail
                    ));
                }
            }
            // what was read comes back as it was
            if e.to_spawn_packet() != *v.spawn || e.update_packet().as_deref() != Some(&*v.update) {
                return Err("the packets kept are not the packets read".into());
            }
            // and written anew from the decoded values: the same name makes it dirty
            let mut fresh = e.clone();
            let name = fresh.name().to_owned();
            if !e.is_opaque() && fresh.set_name(&name).is_ok() {
                let differs = |what: &str, out: &[u8], read: &[u8]| {
                    let at =
                        out.iter().zip(read).position(|(a, b)| a != b).unwrap_or(out.len().min(read.len()));
                    format!(
                        "{} [{}] '{}': the {what} packet written anew differs at byte {at} ({} read, {} written)",
                        e.class_name(),
                        e.section(),
                        e.name(),
                        read.len(),
                        out.len()
                    )
                };
                let spawn = fresh.to_spawn_packet();
                if e.header().version == gz_entities::SPAWN_VERSION {
                    if spawn != *v.spawn {
                        return Err(differs("spawn", &spawn, &v.spawn));
                    }
                } else {
                    // an older packet is written in today's layout, as `Spawn_Write` writes it: that
                    // one must read back to itself
                    let mut up = Entity::from_spawn_packet(&spawn, &settings).map_err(|e| e.to_string())?;
                    let _ = up.set_name(&name);
                    if up.is_opaque() || up.to_spawn_packet() != spawn {
                        return Err(differs("spawn (upgraded, not stable)", &spawn, &v.spawn));
                    }
                    upgraded.fetch_add(1, Ordering::Relaxed);
                }
                // `UPDATE_Write` of the physics block derives its mask from the values (a count of
                // no items, velocities under EPS_S) and always writes the freeze byte as 1: the C++
                // normalises what the spawn compiler wrote, and so does this crate. The rule held to:
                // the update written anew reads back to itself.
                let update = fresh.update_packet().unwrap_or_default();
                if update != *v.update {
                    let again =
                        Entity::from_packets(&spawn, &update, &settings).map_err(|e| e.to_string())?;
                    let mut again = again.clone();
                    let _ = again.set_name(&name);
                    if again.update_packet().unwrap_or_default() != update {
                        return Err(differs("update (not stable)", &update, &v.update));
                    }
                    normalised.fetch_add(1, Ordering::Relaxed);
                }
            }
            Ok(())
        },
    );
    let counted: Vec<String> =
        reasons.iter().map(|(r, n)| format!("{r} {}", n.load(Ordering::Relaxed))).collect();
    println!(
        "opaque: {}; shorter than their class: {}; older packets upgraded: {}; updates the writer normalises: {}",
        counted.join(", "),
        short_packets.load(Ordering::Relaxed),
        upgraded.load(Ordering::Relaxed),
        normalised.load(Ordering::Relaxed)
    );
    finish("all.spawn entities", file.vertices.len(), started, &failed);
}

/// The model of a valid spawn file: its one actor carries `M_SPAWN_OBJECT_ASPLAYER` - the mark
/// the game makes the player of, and the level current from - and the game's actor section says
/// `$player`, the key a new spawn point takes the mark from.
#[test]
fn the_actor_of_all_spawn_is_the_player() {
    let Some(game) = gz_corpus::game() else { return };
    let game = game.unwrap();
    let ltx = system_ltx(&game);
    let settings = LtxSettings(&ltx);
    let file = SpawnFile::parse(Arc::from(game.read_path("spawns/all.spawn").unwrap())).unwrap();
    let actors: Vec<Entity> = file
        .vertices
        .iter()
        .filter_map(|v| Entity::from_packets(&v.spawn, &v.update, &settings).ok())
        .filter(|e| e.class_info().is_some_and(|c| c.class == gz_entities::Class::CreatureActor))
        .collect();
    assert_eq!(actors.len(), 1);
    assert!(actors[0].is_player(), "flags {:#06x}", actors[0].header().flags);
    assert_eq!(settings.r_bool(actors[0].section(), "$player"), Some(true));
    assert!(Entity::create_spawn_point(actors[0].section(), &settings).unwrap().is_player());
}
