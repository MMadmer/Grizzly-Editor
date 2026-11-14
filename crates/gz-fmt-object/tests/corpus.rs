//! The repository's `rawdata` library and the projects: `.object` files every 16th by sorted
//! path (the library is 5.6 GB), all `.skl`, `.skls` and `.anm`. Each reads; written, it reads
//! back to the same value, and a second write gives the same bytes.
//!
//! Byte equality with the file as read is not the rule for an `.object`: the X-Ray SDK writes its
//! surfaces' face lists in the order of the surfaces' addresses and leaves unused vertex-map
//! reference slots uninitialised. How many come
//! back byte for byte is printed.
//!
//! Two things of the library are what the X-Ray SDK makes of them, not faults: a file whose object body
//! chunk is empty or missing is no object (the X-Ray SDK's reader answers null for a chunk of size
//! 0), and an envelope key of a version-1 motion holds its TCB and bezier parameters as floats,
//! which every write quantises to 16 bits over [-32, 32]: those
//! fields read back within half a step.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use gz_corpus::{check_all, finish, stride, walk_ext};
use gz_fmt_object::{Object, ObjectError, read_anm, read_skl, read_skls, write_anm, write_skl, write_skls};

const OBJECT_STRIDE: usize = 16;

/// `EOBJ_CHUNK_OBJECT_BODY`.
const BODY: u32 = 0x7777;

/// Whether the file's top-level chunks hold no object body with bytes in it.
fn body_empty(bytes: &[u8]) -> bool {
    let mut at = 0;
    while let Some(head) = bytes.get(at..at + 8) {
        let id = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) & 0x7fff_ffff;
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        if id == BODY {
            return size == 0;
        }
        at += 8 + size;
    }
    true
}

/// Half a step of `w_float_q16` over [-32, 32], and a float's rounding on top.
const Q16_HALF_STEP: f64 = 0.5 * 64.0 / 65535.0 + 1e-6;

/// The two objects' debug texts are the same, but for the quantised envelope parameters.
fn same_but_quantised(read: &str, back: &str) -> bool {
    let (a, b): (Vec<&str>, Vec<&str>) = (read.lines().collect(), back.lines().collect());
    if a.len() != b.len() {
        return false;
    }
    let mut in_param = false;
    for (x, y) in a.iter().zip(&b) {
        let t = x.trim();
        if t == "param: [" {
            in_param = true;
        } else if t.starts_with(']') {
            in_param = false;
        }
        if x == y {
            continue;
        }
        let (tx, ty) = (t.trim_end_matches(','), y.trim().trim_end_matches(','));
        let value = |s: &str| -> Option<f64> {
            let v = if in_param {
                s
            } else {
                s.split_once(": ").filter(|(k, _)| ["tension", "continuity", "bias"].contains(k))?.1
            };
            v.parse().ok()
        };
        match (value(tx), value(ty)) {
            (Some(p), Some(q)) if (p - q).abs() <= Q16_HALF_STEP => {}
            _ => return false,
        }
    }
    true
}

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
            let o = match Object::read(&bytes) {
                Ok(o) => o,
                // no object for the X-Ray SDK either
                Err(ObjectError::NotAnObject) if body_empty(&bytes) => return Ok(()),
                Err(e) => return Err(e.to_string()),
            };
            let written = o.write();
            if written == bytes {
                same.fetch_add(1, Ordering::Relaxed);
            }
            let back =
                Object::read(&written).map_err(|e| format!("the written object does not read: {e}"))?;
            // by value first; as text where that fails: a bone's joint friction is a NaN in some
            // library files, and a NaN equals nothing
            if back != o
                && format!("{back:?}") != format!("{o:?}")
                && !same_but_quantised(&format!("{o:#?}"), &format!("{back:#?}"))
            {
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
            // the X-Ray SDK writes today's version: a motion of version 6 gains the marks' count
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
