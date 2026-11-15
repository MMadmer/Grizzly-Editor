//! What is wrong with the screens and music. Each line starts with
//! its code: `E4nn` stops a build, `W4nn` does not.
//!
//! `E401` a layer's or a track's file does not exist, `E402` a layer's file is no kind the game
//! shows, `E403` a screen names a track the module does not have, `E404` a shader has no `.ps`
//! (or `.s`) beside it, `E408` the build leaves out a file a screen needs, `E409` a track's file is
//! not an `.ogg`; `W405` a layer nothing was put on, `W406` a screen with no layers, `W407` a level
//! the linked game lacks, `W410` a track that plays nowhere, `W411` a screen asks for a video's
//! sound and has no video; `E412` a playlist names a track the module does not have, `W413` a
//! playlist without tracks, `W414` a playlist's title the game cannot show (outside cp1251);
//! `W415` a track that names a sound and a file (the sound plays), `E416` a track's sound that
//! neither the module nor the linked game has, `W417` a track that still says `copyrighted` or
//! `streamer` (the mark is its sound's), `E418` a track that names no music.

use std::path::Path;

use gz_media::sound::{Facts, Mark};
use gz_mod::{Issue, Spot};

use crate::import::{FileKind, extension, file_kind};
use crate::model::{Screens, Track};

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
    sounds: &dyn Fn(&str) -> Facts,
) -> Vec<String> {
    issues(root, s, outside, levels, sounds).iter().map(Issue::line).collect()
}

/// What a track that still carries the mark is told, at the key's line, and what `screens.set`
/// refuses `copyrighted` and `streamer` with: the mark is the sound's, and where to set it.
/// `asset` is the module's sound asset the track plays, when it plays one.
#[must_use]
pub fn mark_moved(key: &str, track: &Track, asset: Option<&str>) -> String {
    let lead = format!(
        "{key} is no key of a track: whether music is copyrighted, and what plays instead in the streamer mode, is its sound's to say"
    );
    match asset {
        Some(a) => {
            format!("{lead} - tick Copyrighted in its sound asset {a} (its copyrighted and streamer keys)")
        }
        None if track.is_sound() => format!(
            "{lead} - the track plays the game's sound {}, which the game's own list describes",
            track.sound
        ),
        None => format!(
            "{lead} - make the track a sound first (Make It a Sound; screens.set action=track_sound id={}), then tick Copyrighted in its sound asset",
            track.id
        ),
    }
}

/// The problems of `s`, each with the section and the key of `mod.ltx` it is about: a layer's
/// file at its `layer` line, a track's file at its `file` line, a screen without layers at its
/// header. `outside` says whether the build leaves a project path out; `levels` are the linked
/// game's levels, `None` when they are not known (W407 is not checked then); `sounds` says what
/// a sound name is to the streamer mode and whose it is. Blocking: a file is looked for on the disk.
pub fn issues(
    root: &Path,
    s: &Screens,
    outside: &dyn Fn(&str) -> bool,
    levels: Option<&[String]>,
    sounds: &dyn Fn(&str) -> Facts,
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
        if t.is_sound() {
            let at = Spot::key(&section, "sound");
            if !t.file.trim().is_empty() {
                problems.push(Issue::new(
                    "W415",
                    format!(
                        "{tag}it names its music twice, by sound and by file: the sound plays, {} does not",
                        t.file
                    ),
                    file,
                ));
            }
            if sounds(&t.sound).mark == Mark::Missing {
                problems.push(Issue::new(
                    "E416",
                    format!(
                        "{tag}nobody has the sound {}: no sound asset of the module publishes it, and the linked game has none of that name",
                        t.sound
                    ),
                    at,
                ));
            }
        } else if t.file.trim().is_empty() {
            problems.push(Issue::new(
                "E418",
                format!(
                    "{tag}it names no music: sound = <a sound's name> or file = <an .ogg of the project>"
                ),
                Spot::section(&section),
            ));
        } else if extension(&t.file) != ".ogg" {
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
        let named = s.screens.iter().any(|d| d.music == "tracks" && d.tracks.contains(&t.id))
            || s.playlists.iter().any(|p| p.tracks.contains(&t.id));
        if t.play == 0 && !named {
            problems.push(Issue::new(
                "W410",
                format!("{tag}it joins no shuffle and no screen or playlist names it, so it never plays"),
                Spot::section(&section),
            ));
        }
        for key in gz_mod::reference::TRACK_MARK_KEYS {
            if t.extra(key).is_some() {
                let asset = if t.is_sound() { sounds(&t.sound).asset } else { None };
                problems.push(Issue::new(
                    "W417",
                    format!("{tag}{}", mark_moved(key, t, asset.as_deref())),
                    Spot::key(&section, key),
                ));
            }
        }
    }
    for p in &s.playlists {
        let tag = format!("playlist {}: ", p.id);
        let section = format!("playlist.{}", p.id);
        if p.tracks.is_empty() {
            problems.push(Issue::new(
                "W413",
                format!("{tag}it has no tracks, so it plays nothing"),
                Spot::section(&section),
            ));
        }
        for t in &p.tracks {
            if s.track(t).is_none() {
                problems.push(Issue::new(
                    "E412",
                    format!("{tag}it names a track the module does not have: {t}"),
                    Spot::key(&section, "tracks"),
                ));
            }
        }
        if let Some(bad) = gz_mod::text::outside_cp1251(&p.title) {
            problems.push(Issue::new(
                "W414",
                format!("{tag}the game shows the title in cp1251 and has no {bad}: the player sees a look-alike or ?"),
                Spot::key(&section, "title"),
            ));
        }
    }
    problems
}
