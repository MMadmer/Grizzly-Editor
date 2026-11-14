//! What is wrong with the screens and music. Each line starts with
//! its code: `E4nn` stops a build, `W4nn` does not.
//!
//! `E401` a layer's or a track's file does not exist, `E402` a layer's file is no kind the game
//! shows, `E403` a screen names a track the module does not have, `E404` a shader has no `.ps`
//! (or `.s`) beside it, `E408` the build leaves out a file a screen needs, `E409` a track is not
//! an `.ogg`; `W405` a layer nothing was put on, `W406` a screen with no layers, `W407` a level
//! the linked game lacks, `W410` a track that plays nowhere, `W411` a screen asks for a video's
//! sound and has no video.

use std::path::Path;

use crate::import::{FileKind, extension, file_kind};
use crate::model::Screens;

/// The whole path without its extension (`substr(0, rfind('.'))`).
fn without_ext(file: &str) -> &str {
    file.rfind('.').and_then(|d| file.get(..d)).unwrap_or(file)
}

/// The problems of `s`. `outside` says whether the build leaves a project path out; `levels`
/// are the linked game's levels, `None` when they are not known (W407 is not checked then).
/// Blocking: a file is looked for on the disk.
pub fn validate(
    root: &Path,
    s: &Screens,
    outside: &dyn Fn(&str) -> bool,
    levels: Option<&[String]>,
) -> Vec<String> {
    let exists = |rel: &str| root.join(rel.replace('/', "\\")).is_file();
    let mut problems = Vec::new();
    for d in &s.screens {
        let tag = format!("screen {}: ", d.id);
        if d.layers.is_empty() {
            problems.push(format!("W406 {tag}it has no layers, so it is never shown"));
        }
        let mut video = false;
        for (i, l) in d.layers.iter().enumerate() {
            let at = format!("layer {}", i + 1);
            if l.file.is_empty() {
                problems.push(format!("W405 {tag}{at} is empty and is not drawn"));
                continue;
            }
            let kind = file_kind(&l.file);
            video |= kind == FileKind::Video;
            if matches!(kind, FileKind::None | FileKind::Music | FileKind::Convert) {
                problems.push(format!("E402 {tag}{at}: the game shows no {} file", extension(&l.file)));
            } else if !exists(&l.file) {
                problems.push(format!("E401 {tag}{at}: the file does not exist: {}", l.file));
            } else if outside(&l.file) {
                problems.push(format!("E408 {tag}{at}: the build leaves this file out: {}", l.file));
            }
            if kind == FileKind::Shader && exists(&l.file) && !exists(&format!("{}.ps", without_ext(&l.file)))
            {
                problems.push(format!("E404 {tag}{at}: there is no .ps beside this shader"));
            }
            if kind == FileKind::Shader
                && extension(&l.file) == ".ps"
                && !exists(&format!("{}.s", without_ext(&l.file)))
            {
                problems.push(format!("E404 {tag}{at}: there is no .s beside this shader"));
            }
            if !l.texture.is_empty() && !exists(&l.texture) {
                problems.push(format!("E401 {tag}{at}: the shader's texture does not exist: {}", l.texture));
            }
        }
        if d.music == "tracks" {
            for t in &d.tracks {
                if s.track(t).is_none() {
                    problems.push(format!("E403 {tag}it names a track the module does not have: {t}"));
                }
            }
        }
        if d.music == "video" && !video {
            problems.push(format!("W411 {tag}it asks for a video's sound and has no video"));
        }
        if let Some(known) = levels {
            for level in &d.levels {
                if !known.iter().any(|k| k.eq_ignore_ascii_case(level)) {
                    problems.push(format!("W407 {tag}the linked game has no level {level}"));
                }
            }
        }
    }
    for t in &s.tracks {
        let tag = format!("track {}: ", t.id);
        if extension(&t.file) != ".ogg" {
            problems.push(format!("E409 {tag}the game plays .ogg files only: {}", t.file));
        } else if !exists(&t.file) {
            problems.push(format!("E401 {tag}the file does not exist: {}", t.file));
        } else if outside(&t.file) {
            problems.push(format!("E408 {tag}the build leaves this file out: {}", t.file));
        }
        let named = s.screens.iter().any(|d| d.music == "tracks" && d.tracks.contains(&t.id));
        if t.play == 0 && !named {
            problems.push(format!("W410 {tag}it joins no shuffle and no screen names it, so it never plays"));
        }
    }
    problems
}
