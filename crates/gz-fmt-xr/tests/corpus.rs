//! The game's and the editor's own `.xr` libraries: each reads; the ones with a writer write
//! back the bytes they read. Dead Air's `particles.xr` holds 50 effects with Shadow of
//! Chernobyl's 20-byte `TargetColor`.

use std::time::Instant;

use gz_corpus::finish;
use gz_fmt_xr::{
    ActionParams, GameMaterials, LightAnims, Particles, Shaders, ShadersXrlc, SoundEnvironments,
};

/// Reads (and where there is a writer, writes back) one library; `Err` says why not.
fn check(name: &str, bytes: &[u8]) -> Result<String, String> {
    let e = |e: gz_fmt_xr::XrError| e.to_string();
    let same =
        |out: Vec<u8>| if out == bytes { Ok(()) } else { Err("written back, the bytes differ".to_owned()) };
    Ok(match name {
        "shaders.xr" => format!("{} blenders", Shaders::read(bytes).map_err(e)?.blenders.len()),
        "gamemtl.xr" => format!("{} materials", GameMaterials::read(bytes).map_err(e)?.materials.len()),
        "lanims.xr" => {
            let l = LightAnims::read(bytes).map_err(e)?;
            same(l.write())?;
            format!("{} light animations", l.items.len())
        }
        "senvironment.xr" => {
            let s = SoundEnvironments::read(bytes).map_err(e)?;
            same(s.write())?;
            format!("{} sound environments", s.environments.len())
        }
        "shaders_xrlc.xr" => {
            let s = ShadersXrlc::read(bytes).map_err(e)?;
            same(s.write())?;
            format!("{} compiler shaders", s.shaders.len())
        }
        "particles.xr" => {
            let p = Particles::read(bytes).map_err(e)?;
            format!("{} effects, {} groups", p.effects.len(), p.groups.len())
        }
        _ => return Err("not a library this crate reads".into()),
    })
}

const LIBRARIES: [&str; 6] =
    ["shaders.xr", "gamemtl.xr", "lanims.xr", "senvironment.xr", "shaders_xrlc.xr", "particles.xr"];

#[test]
fn the_editors_own_libraries_read_and_write_back() {
    let started = Instant::now();
    let data = gz_corpus::repo().join("gamedata");
    let mut failed = Vec::new();
    for name in LIBRARIES {
        match std::fs::read(data.join(name)).map_err(|e| e.to_string()).and_then(|b| check(name, &b)) {
            Ok(what) => println!("editor {name}: {what}"),
            Err(why) => failed.push(format!("{name}: {why}")),
        }
    }
    finish("editor libraries", LIBRARIES.len(), started, &failed);
}

#[test]
fn the_games_libraries_read_and_write_back() {
    let Some(game) = gz_corpus::game() else { return };
    let started = Instant::now();
    let game = game.unwrap();
    let mut failed = Vec::new();
    let mut checked = 0;
    for name in LIBRARIES {
        let Some(entry) = game.get(name) else { continue };
        checked += 1;
        match game.read(entry).map_err(|e| e.to_string()).and_then(|b| check(name, &b)) {
            Ok(what) => println!("game {name}: {what}"),
            Err(why) => failed.push(format!("{name}: {why}")),
        }
    }
    finish("game libraries", checked, started, &failed);
}

#[test]
fn dead_airs_short_target_colours_read_with_the_engines_defaults() {
    let Some(game) = gz_corpus::game() else { return };
    let game = game.unwrap();
    let p = Particles::read(&game.read_path("particles.xr").unwrap()).unwrap();
    let short: Vec<&str> = p
        .effects
        .iter()
        .filter(|e| {
            e.actions.iter().any(
                |a| matches!(a.params, ActionParams::TargetColor { times_stored, .. } if times_stored < 2),
            )
        })
        .map(|e| e.name.as_str())
        .collect();
    assert_eq!(short.len(), 50, "{short:?}");
    assert_eq!(short.first().copied(), Some("a_ufp_particles\\group_ufp\\atom_flame"));
    // the short form reads as `PATargetColor`'s whole-life window, wherever it stands
    for e in &p.effects {
        for a in &e.actions {
            if let ActionParams::TargetColor { time_from, time_to, times_stored: 0, .. } = a.params {
                assert_eq!((time_from, time_to), (0.0, 1.0), "{}", e.name);
            }
        }
    }
}
