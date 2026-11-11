//! The game's built levels: every `level.cform`, `level.ai`, `level.details`, `level.game`,
//! `level.env_mod`, `level.snd_static`, `level.ps_static` and `level.fog_vol` reads and writes
//! back the bytes it was read from. (A level's scene - the `.level` file and its parts - is
//! `gz-scene`'s, and its corpus test is there.)

use std::time::Instant;

use gz_corpus::{check_all, finish};
use gz_fmt_level::{
    CForm, EnvMods, LevelDetails, LevelGame, LevelGraph, read_fog_volumes, read_static_particles,
    read_static_sounds, write_fog_volumes, write_static_particles, write_static_sounds,
};

fn round_trip(name: &str, bytes: &[u8]) -> Result<(), String> {
    let e = |e: gz_fmt_level::LevelError| e.to_string();
    let out = match name {
        "level.cform" => CForm::read(bytes).map_err(e)?.write(),
        "level.ai" => LevelGraph::read(bytes).map_err(e)?.write().map_err(e)?,
        "level.details" => LevelDetails::read(bytes).map_err(e)?.write(),
        "level.game" => LevelGame::read(bytes).map_err(e)?.write(),
        "level.env_mod" => EnvMods::read(bytes).map_err(e)?.write(),
        "level.snd_static" => write_static_sounds(&read_static_sounds(bytes).map_err(e)?),
        "level.ps_static" => write_static_particles(&read_static_particles(bytes).map_err(e)?),
        "level.fog_vol" => write_fog_volumes(&read_fog_volumes(bytes).map_err(e)?),
        _ => return Ok(()),
    };
    if out != bytes {
        let at = out.iter().zip(bytes).position(|(a, b)| a != b).unwrap_or(out.len().min(bytes.len()));
        return Err(format!(
            "written back, differs from byte {at} ({} read, {} written)",
            bytes.len(),
            out.len()
        ));
    }
    Ok(())
}

const FILES: [&str; 8] = [
    "level.cform",
    "level.ai",
    "level.details",
    "level.game",
    "level.env_mod",
    "level.snd_static",
    "level.ps_static",
    "level.fog_vol",
];

#[test]
fn the_games_level_files_write_back_their_bytes() {
    let Some(game) = gz_corpus::game() else { return };
    let started = Instant::now();
    let game = game.unwrap();
    let entries: Vec<_> = game
        .files()
        .iter()
        .filter(|e| {
            e.key.starts_with("levels/")
                && e.key.matches('/').count() == 2
                && FILES.iter().any(|f| e.key.ends_with(&format!("/{f}")))
        })
        .collect();
    let failed = check_all(
        &entries,
        |e| e.key.clone(),
        |e| {
            let name = e.key.rsplit('/').next().unwrap_or("");
            round_trip(name, &game.read(e).map_err(|e| e.to_string())?)
        },
    );
    finish("level files", entries.len(), started, &failed);
}
