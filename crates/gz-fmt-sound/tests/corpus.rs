//! The game's sounds, every `OGG_STRIDE`th `.ogg` by sorted path: the parameters in the first
//! comment read, and written into the file again, give the file's own bytes.
//!
//! Two exceptions are counted, not failed. A first comment that is no parameter block (an
//! encoder's `ENCODER=` tag, a block cut short) is refused by the reader, as the game refuses it
//! (`CSoundRender_Source::LoadWave` logs "Invalid ogg-comment version" and plays with defaults).
//! A block with bytes after its 24 is written back as the 24 the game reads; the file must then
//! keep its parameters and write back to itself.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use gz_corpus::{check_all, finish, stride};
use gz_fmt_sound::{SoundError, read_ogg_params, write_ogg_params};

const OGG_STRIDE: usize = 4;

#[test]
fn sound_parameters_read_and_write_back() {
    let Some(game) = gz_corpus::game() else { return };
    let started = Instant::now();
    let game = game.unwrap();
    let all = game.with_ext(&["ogg"]);
    let total = all.len();
    let sample = stride(all, OGG_STRIDE);
    let without = AtomicUsize::new(0);
    let unreadable = AtomicUsize::new(0);
    let rebuilt = AtomicUsize::new(0);
    let failed = check_all(
        &sample,
        |e| e.key.clone(),
        |e| {
            let bytes = game.read(e).map_err(|e| e.to_string())?;
            let params = match read_ogg_params(&bytes) {
                Ok(Some(p)) => p,
                Ok(None) => {
                    without.fetch_add(1, Ordering::Relaxed);
                    return Ok(());
                }
                Err(SoundError::CommentVersion(_) | SoundError::Chunk(_)) => {
                    unreadable.fetch_add(1, Ordering::Relaxed);
                    return Ok(());
                }
                Err(e) => return Err(e.to_string()),
            };
            let out = write_ogg_params(&bytes, &params).map_err(|e| e.to_string())?;
            if out != bytes {
                let kept = read_ogg_params(&out).map_err(|e| e.to_string())? == Some(params);
                let again = write_ogg_params(&out, &params).map_err(|e| e.to_string())?;
                if !kept || again != out {
                    let at = out
                        .iter()
                        .zip(&bytes)
                        .position(|(a, b)| a != b)
                        .unwrap_or(out.len().min(bytes.len()));
                    return Err(format!(
                        "written back, differs from byte {at} ({} read, {} written)",
                        bytes.len(),
                        out.len()
                    ));
                }
                rebuilt.fetch_add(1, Ordering::Relaxed);
            }
            Ok(())
        },
    );
    println!(
        "ogg: every {OGG_STRIDE}th of {total}; {} without parameters, {} the game cannot read, {} with a longer block",
        without.load(Ordering::Relaxed),
        unreadable.load(Ordering::Relaxed),
        rebuilt.load(Ordering::Relaxed)
    );
    finish("ogg", sample.len(), started, &failed);
}
