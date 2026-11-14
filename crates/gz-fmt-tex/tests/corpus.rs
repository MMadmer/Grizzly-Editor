//! Texture descriptions and DDS headers. Every `THM_STRIDE`th `.thm` of the repository (22000
//! files) and every one of the projects reads and writes back its own bytes; every `.dds` of the
//! game and of the repository reads its header (only the header is read from disk).
//!
//! A description another tool wrote with chunk sizes that disagree with what they hold (a
//! converted classic mod has such) is written in the X-Ray SDK's layout, as the X-Ray SDK's saver
//! writes it: that text must then write back to itself. One whose chunk list runs past the
//! file's end is refused, as damage.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use gz_corpus::{check_all, finish, stride, walk_ext};
use gz_fmt_tex::{
    Thm, read_dds_info, read_thm, write_group_thm, write_object_thm, write_sound_thm, write_texture_thm,
};

const THM_STRIDE: usize = 8;
/// A DDS header with the DX10 extension.
const DDS_HEAD: usize = 4 + 124 + 20;

fn write(thm: &Thm) -> Result<Vec<u8>, String> {
    Ok(match thm {
        Thm::Texture(t) => write_texture_thm(t).map_err(|e| e.to_string())?,
        Thm::Object(o) => write_object_thm(o).map_err(|e| e.to_string())?,
        Thm::Group(g) => write_group_thm(g).map_err(|e| e.to_string())?,
        Thm::Sound(s) => write_sound_thm(s),
    })
}

/// Whether the top-level chunk headers tile the file exactly.
fn chunks_tile(bytes: &[u8]) -> bool {
    let mut at = 0usize;
    while at < bytes.len() {
        let Some(size) =
            bytes.get(at + 4..at + 8).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]) as usize)
        else {
            return false;
        };
        at = at.saturating_add(8).saturating_add(size);
    }
    at == bytes.len()
}

#[derive(Default)]
struct Counts {
    same: AtomicUsize,
    normalised: AtomicUsize,
    damaged: AtomicUsize,
}

fn thm_round_trip(bytes: &[u8], n: &Counts) -> Result<(), String> {
    let thm = match read_thm(bytes) {
        Ok(t) => t,
        Err(_) if !chunks_tile(bytes) => {
            n.damaged.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
        Err(e) => return Err(e.to_string()),
    };
    let out = write(&thm)?;
    if out == bytes {
        n.same.fetch_add(1, Ordering::Relaxed);
        return Ok(());
    }
    let again = write(&read_thm(&out).map_err(|e| format!("the written description does not read: {e}"))?)?;
    if again != out {
        let at = out.iter().zip(bytes).position(|(a, b)| a != b).unwrap_or(out.len().min(bytes.len()));
        return Err(format!(
            "written back, differs from byte {at} ({} read, {} written)",
            bytes.len(),
            out.len()
        ));
    }
    n.normalised.fetch_add(1, Ordering::Relaxed);
    Ok(())
}

#[test]
fn every_description_writes_back_its_bytes() {
    let started = Instant::now();
    let repo = gz_corpus::repo();
    let mut library: Vec<PathBuf> = walk_ext(&repo.join("rawdata"), &["thm"]);
    library.extend(walk_ext(&repo.join("gamedata"), &["thm"]));
    let total = library.len();
    let mut files = stride(library, THM_STRIDE);
    for p in gz_corpus::projects().unwrap_or_default() {
        files.extend(walk_ext(&p, &["thm"]));
    }
    let n = Counts::default();
    let failed = check_all(
        &files,
        |p| p.display().to_string(),
        |p| thm_round_trip(&std::fs::read(p).map_err(|e| e.to_string())?, &n),
    );
    println!(
        "thm: every {THM_STRIDE}th of {total} in the repository, all of the projects; {} byte for byte, {} in the X-Ray SDK's layout, {} damaged",
        n.same.load(Ordering::Relaxed),
        n.normalised.load(Ordering::Relaxed),
        n.damaged.load(Ordering::Relaxed)
    );
    finish("thm", files.len(), started, &failed);
}

fn dds_check(bytes: &[u8]) -> Result<(), String> {
    // a header whose size field is not 124 is refused, as DirectXTex (the game's loader)
    // refuses it: Dead Air's `internal_light_torch_r2.dds` says 128
    if bytes.starts_with(b"DDS ") && bytes.get(4..8) != Some(&124u32.to_le_bytes()[..]) {
        return if read_dds_info(bytes).is_err() {
            Ok(())
        } else {
            Err("a bad header size was taken".into())
        };
    }
    let info = read_dds_info(bytes).map_err(|e| e.to_string())?;
    if info.width == 0 || info.height == 0 || info.mips == 0 {
        return Err(format!("an empty picture: {info:?}"));
    }
    Ok(())
}

#[test]
fn dds_headers_read() {
    let started = Instant::now();
    let repo = gz_corpus::repo();
    let local = walk_ext(&repo.join("gamedata"), &["dds"]);
    let mut failed = check_all(
        &local,
        |p| p.display().to_string(),
        |p| {
            use std::io::Read;
            let mut head = Vec::with_capacity(DDS_HEAD);
            std::fs::File::open(p)
                .and_then(|f| f.take(DDS_HEAD as u64).read_to_end(&mut head))
                .map_err(|e| e.to_string())?;
            dds_check(&head)
        },
    );
    let mut checked = local.len();
    if let Some(game) = gz_corpus::game() {
        let game = game.unwrap();
        let all = game.with_ext(&["dds"]);
        failed.extend(check_all(
            &all,
            |e| e.key.clone(),
            |e| dds_check(&game.read_head(e, DDS_HEAD).map_err(|e| e.to_string())?),
        ));
        checked += all.len();
    }
    finish("dds", checked, started, &failed);
}
