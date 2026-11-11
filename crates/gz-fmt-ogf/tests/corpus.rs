//! The game's models and motions: every `.ogf` and `.omf` reads, with all its children, vertices,
//! bones and motions. The crate has no writer for a model as read (its writers are the
//! exporters), so reading is what is checked.

use std::time::Instant;

use gz_corpus::{check_all, finish, stride};
use gz_fmt_ogf::{Visual, read_omf};

const OGF_STRIDE: usize = 1;

/// Whether the motion parameters (chunk 15) hold one part `default` whose bones are the
/// indices 0, 1, 2, ... without names.
fn nameless_default_part(bytes: &[u8]) -> bool {
    let r = gz_chunk::Reader::new(bytes);
    let Ok(Some(p)) = r.sub(15) else { return false };
    let d = p.data();
    let Some(rest) = d.get(4..).and_then(|b| b.strip_prefix(b"default\0")) else { return false };
    let (Some(&a), Some(&b)) = (rest.first(), rest.get(1)) else { return false };
    let n = u16::from_le_bytes([a, b]) as usize;
    (0..n.min(4)).all(|i| rest.get(2 + i * 4..6 + i * 4) == Some(&(i as u32).to_le_bytes()[..]))
}

#[test]
fn the_games_models_and_motions_read() {
    let Some(game) = gz_corpus::game() else { return };
    let started = Instant::now();
    let game = game.unwrap();
    let models = game.with_ext(&["ogf"]);
    let total = models.len();
    let mut files = stride(models, OGF_STRIDE);
    files.extend(game.with_ext(&["omf"]));
    let nameless = std::sync::atomic::AtomicUsize::new(0);
    let failed = check_all(
        &files,
        |e| e.key.clone(),
        |e| {
            let bytes = game.read(e).map_err(|e| e.to_string())?;
            let read = if e.key.ends_with(".omf") {
                read_omf(&bytes).map(|_| ())
            } else {
                Visual::read(&bytes).map(|_| ())
            };
            match read {
                Ok(()) => Ok(()),
                // the C++ exporter's default part: bone indices without names (ExportSkeleton.cpp,
                // pinned by the crate's export tests); the engine reads a name before each index
                // and cannot load such a file either
                Err(_) if nameless_default_part(&bytes) => {
                    nameless.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Ok(())
                }
                Err(err) => Err(err.to_string()),
            }
        },
    );
    println!(
        "models: every {OGF_STRIDE}th of {total}, and every motion file; {} carry the nameless default part",
        nameless.load(std::sync::atomic::Ordering::Relaxed)
    );
    finish("ogf and omf", files.len(), started, &failed);
}
