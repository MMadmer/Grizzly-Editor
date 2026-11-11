//! The repository's `rawdata` library and the projects: `.object` files every 16th by sorted
//! path (the library is 5.6 GB), all `.skl`, `.skls` and `.anm`. Each reads; written, it reads
//! back to the same value, and a second write gives the same bytes.
//!
//! Byte equality with the file as read is not the rule for an `.object`: the C++ writes its
//! surfaces' face lists in the order of the surfaces' addresses and leaves unused vertex-map
//! reference slots uninitialised (docs/rust/STATE.md, "From the object/mesh port"). How many come
//! back byte for byte is printed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use gz_corpus::{check_all, finish, stride, walk_ext};
use gz_fmt_object::{Object, read_anm, read_skl, read_skls, write_anm, write_skl, write_skls};

const OBJECT_STRIDE: usize = 16;

fn files(exts: &[&str]) -> Vec<PathBuf> {
    let mut out = walk_ext(&gz_corpus::repo().join("rawdata"), exts);
    for p in gz_corpus::projects().unwrap_or_default() {
        out.extend(walk_ext(&p, exts));
    }
    out
}

#[test]
fn objects_read_write_and_read_back() {
    let started = Instant::now();
    let all = files(&["object"]);
    let total = all.len();
    let sample = stride(all, OBJECT_STRIDE);
    let same = AtomicUsize::new(0);
    let failed = check_all(
        &sample,
        |p| p.display().to_string(),
        |p| {
            let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
            let o = Object::read(&bytes).map_err(|e| e.to_string())?;
            let written = o.write();
            if written == bytes {
                same.fetch_add(1, Ordering::Relaxed);
            }
            let back =
                Object::read(&written).map_err(|e| format!("the written object does not read: {e}"))?;
            // compared as text: a bone's joint friction is a NaN in some library files
            if format!("{back:?}") != format!("{o:?}") {
                return Err("the written object reads to another value".into());
            }
            if back.write() != written {
                return Err("a second write gives other bytes".into());
            }
            Ok(())
        },
    );
    println!(
        "objects: every {OBJECT_STRIDE}th of {total}; {} of {} byte for byte as read",
        same.load(Ordering::Relaxed),
        sample.len()
    );
    finish("objects", sample.len(), started, &failed);
}

/// The version word of a skeletal motion: after its name and three numbers.
fn smotion_version(body: &[u8]) -> Option<u16> {
    let name_end = body.iter().position(|&b| b == 0)?;
    let at = name_end + 1 + 12;
    Some(u16::from_le_bytes([*body.get(at)?, *body.get(at + 1)?]))
}

#[test]
fn motions_read_and_write_back() {
    let started = Instant::now();
    let all = files(&["skl", "skls", "anm"]);
    let upgraded = AtomicUsize::new(0);
    let failed = check_all(
        &all,
        |p| p.display().to_string(),
        |p| {
            let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
            let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            let (written, again) = match ext.as_str() {
                "skl" => {
                    let m = read_skl(&bytes).map_err(|e| e.to_string())?;
                    let w = write_skl(&m);
                    let again = write_skl(&read_skl(&w).map_err(|e| e.to_string())?);
                    (w, again)
                }
                "skls" => {
                    let m = read_skls(&bytes).map_err(|e| e.to_string())?;
                    let w = write_skls(&m);
                    let again = write_skls(&read_skls(&w).map_err(|e| e.to_string())?);
                    (w, again)
                }
                _ => {
                    let m = read_anm(&bytes).map_err(|e| e.to_string())?;
                    let w = write_anm(&m);
                    let again = write_anm(&read_anm(&w).map_err(|e| e.to_string())?);
                    (w, again)
                }
            };
            if again != written {
                return Err("a second write gives other bytes".into());
            }
            // `CSMotion::Save` writes today's version: a motion of version 6 gains the marks' count
            // (4 bytes); only a file of today's version must come back byte for byte
            let current = ext != "skl" || smotion_version(bytes.get(8..).unwrap_or(&[])) == Some(7);
            if !current {
                upgraded.fetch_add(1, Ordering::Relaxed);
            } else if written != bytes {
                let at = written
                    .iter()
                    .zip(&bytes)
                    .position(|(a, b)| a != b)
                    .unwrap_or(written.len().min(bytes.len()));
                return Err(format!(
                    "written back, differs from byte {at} ({} read, {} written)",
                    bytes.len(),
                    written.len()
                ));
            }
            Ok(())
        },
    );
    println!("motions: {} older .skl files written in today's version", upgraded.load(Ordering::Relaxed));
    finish("motions", all.len(), started, &failed);
}
