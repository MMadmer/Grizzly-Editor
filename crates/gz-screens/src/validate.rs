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

use gz_mod::{Issue, Spot};

use crate::import::{FileKind, extension, file_kind};
use crate::model::Screens;

/// The whole path without its extension (`substr(0, rfind('.'))`).
fn without_ext(file: &str) -> &str {
    file.rfind('.').and_then(|d| file.get(..d)).unwrap_or(file)
}

/// The problems of `s` as lines, each starting with its code (see [`issues`]).
pub fn validate(
    root: &Path,
    s: &Screens,
    outside: &dyn Fn(&str) -> bool,
    levels: Option<&[String]>,
) -> Vec<String> {
    issues(root, s, outside, levels).iter().map(Issue::line).collect()
}

/// The problems of `s`, each with the section and the key of `mod.ltx` it is about: a layer's
/// file at its `layer` line, a track's file at its `file` line, a screen without layers at its
/// header. `outside` says whether the build leaves a project path out; `levels` are the linked
/// game's levels, `None` when they are not known (W407 is not checked then). Blocking: a file is
/// looked for on the disk.
pub fn issues(
    root: &Path,
    s: &Screens,
    outside: &dyn Fn(&str) -> bool,
    levels: Option<&[String]>,
) -> Vec<Issue> {
    let exists = |rel: &str| root.join(rel.replace('/', "\\")).is_file();
    let mut problems = Vec::new();
    for d in &s.screens {
        let tag = format!("screen {}: ", d.id);
        let section = format!("screen.{}", d.id);
        let at = |key: &str| Spot::key(&section, key);
        if d.layers.is_empty() {
            problems.push(Issue::new(
                "W406",
                format!("{tag}it has no layers, so it is never shown"),
                Spot::section(&section),
            ));
        }
        let mut video = false;
        for (i, l) in d.layers.iter().enumerate() {
            let (n, layer) = (format!("layer {}", i + 1), Spot::within(&section, "layer", i, None));
            let mut said = |code: &'static str, text: String, spot: &Spot| {
                problems.push(Issue::new(code, format!("{tag}{text}"), spot.clone()));
            };
            if l.file.is_empty() {
                said("W405", format!("{n} is empty and is not drawn"), &layer);
                continue;
            }
            let kind = file_kind(&l.file);
            video |= kind == FileKind::Video;
            if matches!(kind, FileKind::None | FileKind::Music | FileKind::Convert) {
                said("E402", format!("{n}: the game shows no {} file", extension(&l.file)), &layer);
            } else if !exists(&l.file) {
                said("E401", format!("{n}: the file does not exist: {}", l.file), &layer);
            } else if outside(&l.file) {
                said("E408", format!("{n}: the build leaves this file out: {}", l.file), &layer);
            }
            if kind == FileKind::Shader && exists(&l.file) && !exists(&format!("{}.ps", without_ext(&l.file)))
            {
                said("E404", format!("{n}: there is no .ps beside this shader"), &layer);
            }
            if kind == FileKind::Shader
                && extension(&l.file) == ".ps"
                && !exists(&format!("{}.s", without_ext(&l.file)))
            {
                said("E404", format!("{n}: there is no .s beside this shader"), &layer);
            }
            if !l.texture.is_empty() && !exists(&l.texture) {
                let texture = Spot::within(&section, "layer", i, Some("texture"));
                said("E401", format!("{n}: the shader's texture does not exist: {}", l.texture), &texture);
            }
        }
        if d.music == "tracks" {
            for t in &d.tracks {
                if s.track(t).is_none() {
                    problems.push(Issue::new(
                        "E403",
                        format!("{tag}it names a track the module does not have: {t}"),
                        at("music"),
                    ));
                }
            }
        }
        if d.music == "video" && !video {
            problems.push(Issue::new(
                "W411",
                format!("{tag}it asks for a video's sound and has no video"),
                at("music"),
            ));
        }
        if let Some(known) = levels {
            for level in &d.levels {
                if !known.iter().any(|k| k.eq_ignore_ascii_case(level)) {
                    problems.push(Issue::new(
                        "W407",
                        format!("{tag}the linked game has no level {level}"),
                        at("levels"),
                    ));
                }
            }
        }
    }
    for t in &s.tracks {
        let tag = format!("track {}: ", t.id);
        let section = format!("track.{}", t.id);
        let file = Spot::key(&section, "file");
        if extension(&t.file) != ".ogg" {
            problems.push(Issue::new(
                "E409",
                format!("{tag}the game plays .ogg files only: {}", t.file),
                file,
            ));
        } else if !exists(&t.file) {
            problems.push(Issue::new("E401", format!("{tag}the file does not exist: {}", t.file), file));
        } else if outside(&t.file) {
            problems.push(Issue::new(
                "E408",
                format!("{tag}the build leaves this file out: {}", t.file),
                file,
            ));
        }
        let named = s.screens.iter().any(|d| d.music == "tracks" && d.tracks.contains(&t.id));
        if t.play == 0 && !named {
            problems.push(Issue::new(
                "W410",
                format!("{tag}it joins no shuffle and no screen names it, so it never plays"),
                Spot::section(&section),
            ));
        }
    }
    problems
}
