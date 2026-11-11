//! The scenes of the repository's `rawdata\levels` and of the projects: every `.level` reads
//! with its parts and is saved again. Read without the game's configs, a spawn point keeps its
//! packet as it was read, as the editor does then.
//!
//! - A project's scene, written by this fork of the editor, comes back byte for byte - in the
//!   code page each file was read in (a cp1251 part stays cp1251).
//! - The repository's levels were written by the 2015 SDK, whose CRT printed `%f` differently
//!   (17 digits, then zeros) and from floats that do not read back to the same digits: the C++
//!   editor writes them anew too. Their rule is the fixed point: the text written, read and
//!   written again, is the same.
//! - Every part's GUID is the scene's: a GUID half above `i64::MAX` reads whole (the reader once
//!   saturated it, and 20 of the repository's levels were refused for a "stale" part).

use std::path::{Path, PathBuf};
use std::time::Instant;

use gz_corpus::{check_all, finish};
use gz_scene::ObjClass;
use gz_scene::io::{PartFiles, parse_scene, part_dir, write_scene};

fn scenes(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> =
        rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "level")).collect();
    out.sort();
    out
}

/// The part files beside a scene, per class, `x.part` then `x.part1` ...
fn parts(scene: &Path) -> PartFiles {
    let dir = part_dir(scene);
    let mut p = PartFiles::default();
    for class in ObjClass::ALL {
        let mut mine = Vec::new();
        for i in 0.. {
            let name =
                if i == 0 { format!("{}.part", class.name()) } else { format!("{}.part{i}", class.name()) };
            let path = dir.join(name);
            match std::fs::read(&path) {
                Ok(b) => mine.push((path, b)),
                Err(_) => break,
            }
        }
        p.files.push((class, mine));
    }
    p
}

fn check(scene: &Path, strict: bool) -> Result<(), String> {
    let main = std::fs::read(scene).map_err(|e| e.to_string())?;
    let parts = parts(scene);
    let state = parse_scene(&main, scene, &parts, None).map_err(|e| e.to_string())?.state;
    let files = write_scene(&state, scene).map_err(|e| e.to_string())?;
    let mut read: Vec<(PathBuf, Vec<u8>)> = vec![(scene.to_path_buf(), main)];
    read.extend(parts.files.into_iter().flat_map(|(_, f)| f));
    let written = |path: &Path| files.iter().find(|f| f.path == path).and_then(|f| f.bytes.clone());
    // the fixed point: what was written, read and written again
    let again = if strict {
        None
    } else {
        let mut parts2 = PartFiles::default();
        for class in ObjClass::ALL {
            let mine: Vec<(PathBuf, Vec<u8>)> = files
                .iter()
                .filter(|f| {
                    f.path.parent() == Some(&part_dir(scene))
                        && f.path.file_name().is_some_and(|n| {
                            n.to_string_lossy().starts_with(&format!("{}.part", class.name()))
                        })
                })
                .filter_map(|f| f.bytes.clone().map(|b| (f.path.clone(), b)))
                .collect();
            parts2.files.push((class, mine));
        }
        let main2 = written(scene).unwrap_or_default();
        let state2 = parse_scene(&main2, scene, &parts2, None)
            .map_err(|e| format!("the written scene does not read: {e}"))?
            .state;
        Some(write_scene(&state2, scene).map_err(|e| e.to_string())?)
    };
    for (path, bytes) in &read {
        let Some(out) = written(path) else {
            return Err(format!("{} is not written again", path.display()));
        };
        let expected: &[u8] = match &again {
            None => bytes,
            Some(files2) => {
                files2.iter().find(|f| f.path == *path).and_then(|f| f.bytes.as_deref()).unwrap_or(&[])
            }
        };
        if out != expected {
            let at =
                out.iter().zip(expected).position(|(a, b)| a != b).unwrap_or(out.len().min(expected.len()));
            return Err(format!(
                "{}: written back, differs from byte {at} ({} expected, {} written)",
                path.display(),
                expected.len(),
                out.len()
            ));
        }
    }
    Ok(())
}

#[test]
fn every_scene_writes_back_the_files_it_was_read_from() {
    let started = Instant::now();
    let mut all: Vec<(PathBuf, bool)> =
        scenes(&gz_corpus::repo().join("rawdata").join("levels")).into_iter().map(|p| (p, false)).collect();
    for p in gz_corpus::projects().unwrap_or_default() {
        all.extend(scenes(&p.join("rawdata").join("levels")).into_iter().map(|s| (s, true)));
    }
    let failed = check_all(&all, |(p, _)| p.display().to_string(), |(p, strict)| check(p, *strict));
    finish("scenes", all.len(), started, &failed);
}
