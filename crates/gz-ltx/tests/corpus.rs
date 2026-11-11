//! The real `.ltx` files: the game's, the projects', the repository's. The lossless document
//! writes the bytes it read; the engine's tree loads `system.ltx` and the other roots the
//! editor reads, with every include, without an error.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

use gz_base::text::{cp1251_to_utf8, utf8_to_cp1251};
use gz_corpus::{Game, check_all, finish, key, walk_ext};
use gz_ltx::{Dialect, Loader, Ltx, LtxDoc, ParsedFile, parse_file};
use rayon::prelude::*;

fn round_trip(bytes: &[u8]) -> Result<(), String> {
    let text = cp1251_to_utf8(bytes);
    let back = LtxDoc::parse(&text).to_string();
    let (out, _) = utf8_to_cp1251(&back);
    if out.as_ref() == bytes {
        return Ok(());
    }
    let at = out.iter().zip(bytes).position(|(a, b)| a != b).unwrap_or(out.len().min(bytes.len()));
    Err(format!("differs from byte {at} ({} bytes read, {} written)", bytes.len(), out.len()))
}

#[test]
fn every_ltx_writes_back_the_bytes_it_read() {
    let started = Instant::now();
    // the repository's and the projects' files, then the game's
    let mut files: Vec<PathBuf> = walk_ext(&gz_corpus::repo().join("gamedata"), &["ltx"]);
    files.extend(walk_ext(&gz_corpus::repo().join("tools").join("qa").join("fixtures"), &["ltx"]));
    for p in gz_corpus::projects().unwrap_or_default() {
        files.extend(walk_ext(&p, &["ltx"]));
    }
    let failed = check_all(
        &files,
        |p| p.display().to_string(),
        |p| round_trip(&std::fs::read(p).map_err(|e| e.to_string())?),
    );
    finish("ltx on disk", files.len(), started, &failed);

    let Some(game) = gz_corpus::game() else { return };
    let started = Instant::now();
    let game = game.unwrap();
    let entries = game.with_ext(&["ltx"]);
    let failed =
        check_all(&entries, |e| e.key.clone(), |e| round_trip(&game.read(e).map_err(|e| e.to_string())?));
    finish("ltx of the game", entries.len(), started, &failed);
}

/// The game's configs parsed, as the engine's file system lists them.
struct Configs {
    files: BTreeMap<String, ParsedFile>,
}

impl Configs {
    fn read(game: &Game, dialect: Dialect) -> Self {
        let entries: Vec<_> = game
            .files()
            .iter()
            .filter(|e| e.key.starts_with("configs/") && e.key.ends_with(".ltx"))
            .collect();
        let files = entries
            .par_iter()
            .filter_map(|e| {
                let bytes = game.read(e).ok()?;
                Some((e.key.clone(), parse_file(&cp1251_to_utf8(&bytes), dialect)))
            })
            .collect();
        Self { files }
    }
}

impl Loader for Configs {
    fn file(&self, path: &str) -> Option<&ParsedFile> {
        self.files.get(&key(path))
    }

    fn list(&self, dir: &str, mask: &str) -> Vec<String> {
        let mut prefix = key(dir);
        prefix.push('/');
        let (head, tail) = mask.split_once('*').unwrap_or((mask, ""));
        let (head, tail) = (head.to_lowercase(), tail.to_lowercase());
        self.files
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix))
            .filter(|rel| {
                let name = rel.rsplit('/').next().unwrap_or(rel);
                name.len() >= head.len() + tail.len() && name.starts_with(&head) && name.ends_with(&tail)
            })
            .map(|rel| rel.replace('/', "\\"))
            .collect()
    }
}

#[test]
fn the_engine_tree_loads_every_root_the_editor_reads() {
    let Some(game) = gz_corpus::game() else { return };
    let started = Instant::now();
    let game = game.unwrap();
    let mut checked = 0;
    let mut failed = Vec::new();
    for dialect in [Dialect::Game, Dialect::Editor] {
        let configs = Configs::read(&game, dialect);
        for root in [
            "configs\\system.ltx",
            "configs\\game_levels.ltx",
            "configs\\localization.ltx",
            "configs\\creatures\\game_relations.ltx",
            "configs\\misc\\simulation.ltx",
            "configs\\misc\\smart_names.ltx",
            "configs\\misc\\trade\\trade_presets.ltx",
        ] {
            if configs.file(root).is_none() {
                continue;
            }
            checked += 1;
            match Ltx::assemble(root, &configs, dialect) {
                Ok(loaded) => {
                    if root.ends_with("system.ltx") {
                        // the tree holds what the game is made of
                        assert!(loaded.ltx.section_count() > 1000, "{root}: {}", loaded.ltx.section_count());
                        assert!(loaded.ltx.r_string("actor", "class").is_ok(), "{root}: no [actor]");
                    }
                }
                Err(e) => failed.push(format!("{root} ({dialect:?}): {e}")),
            }
        }
    }
    finish("config trees", checked, started, &failed);
}
