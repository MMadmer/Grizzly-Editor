//! A level the linked game already has - a stock level, one its game graph lists - is the game's.
//! The module adds to it (the spawn layer, the overlays the author exports) and the game composes
//! that over its own level on a new game, so none of the steps that compile a level is needed for
//! the module, and Play plays the module instead of a compiled level.
//!
//! The compile steps still run when they are asked for by name: their output is then the
//! author's and ships, replacing the game's own files of the level in every game that mounts the
//! module. What a build left there is named ([`written`], [`replacing`]) as a warning on the level
//! ([`problem`]), and `level.clean` removes exactly that.

use std::path::Path;

use gz_base::Severity;

use crate::service::LevelProblem;
use crate::state::BuildState;
use crate::steps::Step;

/// Whether `level` is a level of the linked game, any case.
#[must_use]
pub fn is_stock(levels: &[String], level: &str) -> bool {
    !level.trim().is_empty() && levels.iter().any(|l| l.eq_ignore_ascii_case(level.trim()))
}

/// What a step makes in the level folder by name, for a record older than its list of files
/// (`*` is any run of characters, `\` a subfolder).
#[must_use]
pub fn known_files(step: Step) -> &'static [&'static str] {
    match step {
        Step::Game => &[
            "level.ltx",
            "level.spawn",
            "level_rs.spawn",
            "level.game",
            "level.env_mod",
            "level.snd_static",
            "level.ps_static",
            "level.fog_vol",
            "meshes\\brkbl#*.ogf",
        ],
        Step::AiMap => &["level.ai"],
        Step::GameGraph => &["level.graph"],
        Step::Geometry => &[
            "build.prj",
            "level_lods.dds",
            "level_lods_nm.dds",
            "level_stat.tga",
            "level_stat.txt",
            "level.hom",
            "level.som",
            "level.wallmarks",
        ],
        Step::Details => &["build.details", "build_details.dds"],
        Step::Light => &[
            "level",
            "level.geom",
            "level.geomx",
            "level.cform",
            "build.lights",
            "build.cform",
            "build.err",
            "lmap#*_1.dds",
            "lmap#*_2.dds",
        ],
        Step::DetailLight => &["level.details"],
        Step::Spawn | Step::SpawnLayer | Step::Piece => &[],
    }
}

/// A name against a pattern of [`known_files`], any case.
fn matches(pattern: &str, name: &str) -> bool {
    let (p, n) = (pattern.to_ascii_lowercase(), name.to_ascii_lowercase());
    match p.split_once('*') {
        Some((head, tail)) => n.len() >= head.len() + tail.len() && n.starts_with(head) && n.ends_with(tail),
        None => p == n,
    }
}

/// The files of the level folder, folder-relative with `\`, one level of subfolders deep (what a
/// step writes there), links not followed.
fn listing(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let Ok(top) = std::fs::read_dir(dir) else { return out };
    for e in top.flatten() {
        let Ok(t) = e.file_type() else { continue };
        let name = e.file_name().to_string_lossy().into_owned();
        if t.is_file() {
            out.push(name);
        } else if t.is_dir() && !name.starts_with('.') {
            let Ok(sub) = std::fs::read_dir(e.path()) else { continue };
            for s in sub.flatten() {
                if s.file_type().is_ok_and(|t| t.is_file()) {
                    out.push(format!("{name}\\{}", s.file_name().to_string_lossy()));
                }
            }
        }
    }
    out
}

/// What the compile steps of `record` wrote into the level folder `dir` and is there now:
/// folder-relative names with `\`, sorted. The steps' own lists, or, for a record older than
/// them, the names the steps make. Blocking.
#[must_use]
pub fn written(record: &BuildState, dir: &Path) -> Vec<String> {
    let there = listing(dir);
    let mut out: Vec<String> = Vec::new();
    for (step, r) in &record.steps {
        if !step.compiles() {
            continue;
        }
        if r.files.is_empty() {
            for p in known_files(*step) {
                out.extend(there.iter().filter(|n| matches(p, n)).cloned());
            }
        } else {
            for f in &r.files {
                let f = f.replace('/', "\\");
                // a file outside the level folder (the spawn's) is not the level's
                if f.to_ascii_lowercase().starts_with("gamedata\\") {
                    continue;
                }
                if let Some(n) = there.iter().find(|n| n.eq_ignore_ascii_case(&f)) {
                    out.push(n.clone());
                }
            }
        }
    }
    out.sort_by_key(|n| n.to_ascii_lowercase());
    out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    out
}

/// Of what a build wrote, the files the game reads - the ones that replace the stock level's
/// own; the build's own files (`build.prj`, `build.state`, ...) never ship.
#[must_use]
pub fn replacing(written: &[String]) -> Vec<String> {
    written.iter().filter(|n| !gz_project::is_level_build_name(n)).cloned().collect()
}

/// The warning on a stock level whose folder holds files the game would take for its own.
#[must_use]
pub fn problem(level: &str, replacing: &[String]) -> Option<LevelProblem> {
    if replacing.is_empty() {
        return None;
    }
    let mut named: Vec<&str> = replacing.iter().map(String::as_str).take(10).collect();
    let more = replacing.len().saturating_sub(named.len());
    let tail = if more > 0 { format!(" and {more} more") } else { String::new() };
    named.sort_unstable();
    Some(LevelProblem {
        step: Step::Game,
        code: "replaces_stock",
        severity: Severity::Warning,
        text: format!(
            "The module replaces {} of {level}'s own files: {}{tail}. An earlier build compiled the level here; Remove Build Files (level.clean) removes what it wrote.",
            replacing.len(),
            named.join(", ")
        ),
        object: None,
        position: None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::state::StepRecord;
    use crate::steps::Quality;

    fn record(files: &[(Step, &[&str])]) -> BuildState {
        let mut r = BuildState::new("l01_escape");
        for (step, list) in files {
            r.steps.insert(
                *step,
                StepRecord {
                    fingerprint: 1,
                    time: String::new(),
                    seconds: 0.0,
                    quality: Quality::Standard,
                    flags: String::new(),
                    library: 0,
                    failed: None,
                    files: list.iter().map(|s| (*s).to_owned()).collect(),
                },
            );
        }
        r
    }

    #[test]
    fn a_stock_level_is_one_the_game_graph_lists() {
        let levels = vec!["l01_escape".to_owned(), "fake_start".to_owned()];
        assert!(is_stock(&levels, "L01_Escape"));
        assert!(!is_stock(&levels, "mp_test"));
        assert!(!is_stock(&levels, " "));
        assert!(!is_stock(&[], "l01_escape"));
    }

    #[test]
    fn what_a_build_wrote_is_found_by_its_list_or_by_the_names_the_steps_make() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        std::fs::create_dir_all(d.join("meshes")).unwrap();
        std::fs::create_dir_all(d.join(".lc_new")).unwrap();
        for f in [
            "level.ai",
            "level.spawn",
            "level.ltx",
            "build.prj",
            "build.state",
            "level_lods.dds",
            "level_stat.txt",
            "meshes\\brkbl#0.ogf",
            "meshes\\brkbl#12.ogf",
            "meshes\\keep.ogf",
            "author_notes.txt",
            ".lc_new\\level",
        ] {
            std::fs::write(d.join(f), b"x").unwrap();
        }
        // a record older than the lists: the names the steps make
        let old =
            record(&[(Step::Game, &[]), (Step::AiMap, &[]), (Step::Geometry, &[]), (Step::SpawnLayer, &[])]);
        let w = written(&old, d);
        assert_eq!(
            w,
            [
                "build.prj",
                "level.ai",
                "level.ltx",
                "level.spawn",
                "level_lods.dds",
                "level_stat.txt",
                "meshes\\brkbl#0.ogf",
                "meshes\\brkbl#12.ogf"
            ]
        );
        assert!(!w.iter().any(|n| n == "author_notes.txt" || n == "meshes\\keep.ogf" || n == "build.state"));
        // what replaces the game's files: not the build's own
        let r = replacing(&w);
        assert_eq!(
            r,
            [
                "level.ai",
                "level.ltx",
                "level.spawn",
                "level_lods.dds",
                "meshes\\brkbl#0.ogf",
                "meshes\\brkbl#12.ogf"
            ]
        );
        let p = problem("l01_escape", &r).unwrap();
        assert_eq!(p.severity, Severity::Warning);
        assert!(
            p.text.starts_with("The module replaces 6 of l01_escape's own files: level.ai,"),
            "{}",
            p.text
        );
        assert!(problem("l01_escape", &[]).is_none());
        // a record with its lists: exactly those, the spawn's file outside the folder left out
        let new = record(&[(Step::AiMap, &["level.ai"]), (Step::Spawn, &["gamedata/spawns/editor.spawn"])]);
        assert_eq!(written(&new, d), ["level.ai"]);
    }
}
