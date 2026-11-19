//! Bringing a library up to date with its game (`docs/GAME_LIBRARY.md`, 4).
//!
//! [`sync`] reads the game's index, finds per kind what is new, changed and gone against the
//! saved state, describes exactly that on the pool it is called in, and writes the cards, the
//! catalogs and the state. Blocking, and safe to stop: the state is saved as it goes, so the
//! next sync goes on where this one stopped.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use gz_vfs::{FileRef, GameFiles};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use serde_json::{Value, json};

use crate::layout::{Layout, engine_name, top_of, write_atomic};
use crate::state::{Entry, Stamps, State, join_stamps};
use crate::{lists, lua, media, models};

/// What a sync reports to whoever runs it.
pub trait Progress: Sync {
    /// The stage the sync is in, for a person.
    fn stage(&self, text: &str);
    /// How far the stage is.
    fn progress(&self, done: usize, total: usize);
    /// Whether the sync was asked to stop.
    fn stopped(&self) -> bool;
}

/// A sync nobody watches.
pub struct Quiet;

impl Progress for Quiet {
    fn stage(&self, _: &str) {}
    fn progress(&self, _: usize, _: usize) {}
    fn stopped(&self) -> bool {
        false
    }
}

/// What to bring up to date.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Everything is described again, whatever the state says.
    pub rebuild: bool,
    /// Only these kinds (`models`, `textures`, `sounds`, `lists`); `None` is all of them.
    pub kinds: Option<Vec<String>>,
    /// Only game paths that start with this (a test's or a narrow sync's): what lies elsewhere
    /// is left as the state has it.
    pub only: Option<String>,
}

impl Options {
    fn takes(&self, kind: &str) -> bool {
        self.kinds.as_ref().is_none_or(|k| k.iter().any(|x| x == kind))
    }

    fn takes_path(&self, path: &str) -> bool {
        self.only.as_ref().is_none_or(|p| path.starts_with(p.as_str()))
    }
}

/// What a sync did to one kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KindReport {
    /// Files of the kind the game holds.
    pub total: usize,
    /// Described in this sync.
    pub made: usize,
    /// Gone from the game, and from the library with it.
    pub removed: usize,
    /// Files that do not read: `(game path, why)`, the first of them.
    pub problems: Vec<(String, String)>,
    pub problem_count: usize,
}

/// What a sync did.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub kinds: BTreeMap<String, KindReport>,
    /// The lists made again.
    pub lists: Vec<String>,
    pub seconds: f32,
    /// The sync was stopped before it finished.
    pub stopped: bool,
}

impl Report {
    /// Whether the sync changed anything of the library.
    #[must_use]
    pub fn changed(&self) -> bool {
        !self.lists.is_empty() || self.kinds.values().any(|k| k.made > 0 || k.removed > 0)
    }

    #[must_use]
    pub fn to_json(&self) -> Value {
        let kinds: serde_json::Map<String, Value> = self
            .kinds
            .iter()
            .map(|(k, r)| {
                (
                    k.clone(),
                    json!({
                        "total": r.total,
                        "made": r.made,
                        "removed": r.removed,
                        "problems": r.problem_count,
                        "first_problems": r.problems.iter().map(|(p, why)| json!({ "file": p, "problem": why })).collect::<Vec<_>>(),
                    }),
                )
            })
            .collect();
        json!({
            "kinds": kinds,
            "lists": self.lists,
            "seconds": (f64::from(self.seconds) * 10.0).round() / 10.0,
            "stopped": self.stopped,
            "changed": self.changed(),
        })
    }
}

/// A kind of the game described a file at a time.
struct FileKind {
    /// The kind's name in the state and in the reports.
    name: &'static str,
    /// The folder of the library its catalogs lie in.
    catalog: &'static str,
    /// The game folder its files lie under, with the slash.
    under: &'static str,
    ext: &'static str,
    version: u32,
    /// What a person reads while it is synced.
    title: &'static str,
}

const FILE_KINDS: [FileKind; 3] = [
    FileKind {
        name: "model",
        catalog: "models",
        under: "meshes/",
        ext: "ogf",
        version: models::VERSION,
        title: "models",
    },
    FileKind {
        name: "texture",
        catalog: "textures",
        under: "textures/",
        ext: "dds",
        version: media::TEXTURE_VERSION,
        title: "textures",
    },
    FileKind {
        name: "sound",
        catalog: "sounds",
        under: "sounds/",
        ext: "ogg",
        version: media::SOUND_VERSION,
        title: "sounds",
    },
];

/// The first problems of a kind kept for the report.
const PROBLEMS_KEPT: usize = 20;
/// The state is saved this often while a kind is described, so a stop loses little.
const SAVE_EVERY: std::time::Duration = std::time::Duration::from_secs(20);

/// What one file became.
enum Made {
    Row(String),
    Problem(String),
}

fn describe(kind: &FileKind, files: &GameFiles, layout: &Layout, f: FileRef<'_>) -> Made {
    let bytes = match files.read_ref(f) {
        Ok(b) => b,
        Err(e) => return Made::Problem(format!("it could not be read: {e}")),
    };
    let name = engine_name(f.path, kind.under);
    match kind.name {
        "model" => match models::describe(&name, f.path, &bytes) {
            Ok(d) => {
                let text = lua::file(&card_comment(files, f.path), &d.card);
                match write_atomic(&layout.model(&name), text.as_bytes()) {
                    Ok(()) => Made::Row(lua::inline(&d.row)),
                    Err(e) => Made::Problem(format!("its card could not be written: {e}")),
                }
            }
            Err(e) => Made::Problem(e),
        },
        "texture" => {
            // the description the game keeps beside the picture
            let thm = format!("{}.thm", f.path.strip_suffix(".dds").unwrap_or(f.path));
            let thm = files.get(&thm).and_then(|t| files.read_ref(t).ok());
            match media::describe_texture(&name, &bytes, thm.as_deref()) {
                Ok(row) => Made::Row(lua::inline(&row)),
                Err(e) => Made::Problem(e),
            }
        }
        _ => match media::describe_sound(&name, &bytes) {
            Ok(row) => Made::Row(lua::inline(&row)),
            Err(e) => Made::Problem(e),
        },
    }
}

/// The comment every file of the library starts with.
fn card_comment(files: &GameFiles, what: &str) -> String {
    format!(
        "Generated by Grizzly Editor from {} of the game at {}.\nDo not edit: it is written again when the game's file changes.",
        what.replace('/', "\\"),
        files.root().display()
    )
}

fn problem_row(name: &str, why: &str) -> String {
    lua::inline(&json!({ "name": name, "problem": why }))
}

/// Writes the catalogs of the top folders named: every row of the state that lies in one, by
/// name.
fn write_catalogs(
    kind: &FileKind,
    files: &GameFiles,
    layout: &Layout,
    rows: &FxHashMap<String, Entry>,
    tops: &BTreeSet<String>,
) -> std::io::Result<()> {
    for top in tops {
        let mut mine: Vec<(&String, &Entry)> =
            rows.iter().filter(|(path, _)| top_of(path, kind.under) == top.as_str()).collect();
        let path = layout.catalog(kind.catalog, top);
        if mine.is_empty() {
            let _ = std::fs::remove_file(&path);
            continue;
        }
        mine.sort_by(|a, b| a.0.cmp(b.0));
        let folder = if top.is_empty() {
            kind.under.trim_end_matches('/').to_owned()
        } else {
            format!("{}{top}", kind.under)
        };
        let mut text = String::new();
        for l in card_comment(files, &folder).lines() {
            text.push_str("-- ");
            text.push_str(l);
            text.push('\n');
        }
        text.push_str(&format!(
            "return {{\n  card = {},\n  kind = \"{}\",\n  {} = {{\n",
            kind.version, kind.catalog, kind.catalog
        ));
        for (_, e) in mine {
            text.push_str("    ");
            text.push_str(&e.row);
            text.push_str(",\n");
        }
        text.push_str("  },\n}\n");
        write_atomic(&path, text.as_bytes())?;
    }
    Ok(())
}

fn sync_kind(
    kind: &FileKind,
    files: &GameFiles,
    stamps: &Stamps,
    layout: &Layout,
    state: &mut State,
    opts: &Options,
    p: &dyn Progress,
) -> Result<KindReport, String> {
    let mut report = KindReport::default();
    let ext = format!(".{}", kind.ext);
    let all: Vec<FileRef<'_>> =
        files.iter().filter(|f| f.path.starts_with(kind.under) && f.path.ends_with(&ext)).collect();
    report.total = all.len();
    if state.versions.get(kind.name) != Some(&kind.version) || opts.rebuild {
        // another version of the descriptions, or everything asked again: nothing made stays
        if opts.only.is_none() {
            state.files.remove(kind.name);
        } else if let Some(rows) = state.files.get_mut(kind.name) {
            rows.retain(|path, _| !opts.takes_path(path));
        }
        if opts.only.is_none() {
            state.versions.insert(kind.name.to_owned(), kind.version);
        }
    }
    // the kind's rows are worked on outside the state, and put back whenever it is saved
    let mut held = state.files.remove(kind.name).unwrap_or_default();
    let mut tops: BTreeSet<String> = BTreeSet::new();
    // a catalog the state has rows of and the library no file: a sync that was killed between
    // a save of the state and its catalogs, or a file somebody removed - it is written again
    let known: BTreeSet<&str> = held.keys().map(|path| top_of(path, kind.under)).collect();
    for top in known {
        if !layout.catalog(kind.catalog, top).is_file() {
            tops.insert(top.to_owned());
        }
    }
    // what is gone
    let present: rustc_hash::FxHashSet<&str> = all.iter().map(|f| f.path).collect();
    let gone: Vec<String> =
        held.keys().filter(|p| !present.contains(p.as_str()) && opts.takes_path(p)).cloned().collect();
    for path in gone {
        held.remove(&path);
        tops.insert(top_of(&path, kind.under).to_owned());
        if kind.name == "model" {
            let _ = std::fs::remove_file(layout.model(&engine_name(&path, kind.under)));
        }
        report.removed += 1;
    }
    // what is new or another file now
    let stale: Vec<(FileRef<'_>, String)> = all
        .par_iter()
        .filter(|f| opts.takes_path(f.path))
        .filter_map(|f| {
            let stamp = stamps.of_file(*f);
            (held.get(f.path).map(|e| e.stamp.as_str()) != Some(stamp.as_str())).then_some((*f, stamp))
        })
        .collect();
    let total = stale.len();
    let done = AtomicUsize::new(0);
    let mut last_save = Instant::now();
    // in runs, so the state is saved and the stop is heard between them
    for run in stale.chunks(256) {
        if p.stopped() {
            break;
        }
        let made: Vec<(&str, &String, Made)> = run
            .par_iter()
            .map(|(f, stamp)| {
                let m = describe(kind, files, layout, *f);
                let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                if n.is_multiple_of(16) {
                    p.progress(n, total);
                }
                (f.path, stamp, m)
            })
            .collect();
        for (path, stamp, m) in made {
            let row = match m {
                Made::Row(row) => row,
                Made::Problem(why) => {
                    report.problem_count += 1;
                    if report.problems.len() < PROBLEMS_KEPT {
                        report.problems.push((path.to_owned(), why.clone()));
                    }
                    problem_row(&engine_name(path, kind.under), &why)
                }
            };
            held.insert(path.to_owned(), Entry { stamp: stamp.clone(), row });
            tops.insert(top_of(path, kind.under).to_owned());
            report.made += 1;
        }
        if last_save.elapsed() >= SAVE_EVERY {
            // the catalogs first, then the state that says they are made: a run that is killed
            // between the two describes a few files again, and none is left out of a catalog
            if write_catalogs(kind, files, layout, &held, &tops).is_ok() {
                tops.clear();
                state.files.insert(kind.name.to_owned(), std::mem::take(&mut held));
                let _ = write_atomic(&layout.state(), state.to_text().as_bytes());
                held = state.files.remove(kind.name).unwrap_or_default();
            }
            last_save = Instant::now();
        }
    }
    p.progress(total, total);
    let written = write_catalogs(kind, files, layout, &held, &tops);
    state.files.insert(kind.name.to_owned(), held);
    written.map_err(|e| format!("a catalog of the {} could not be written: {e}", kind.title))?;
    Ok(report)
}

/// A list that is made of a few files of the game: its name, the files, its version.
const LISTS: [(&str, &[&str], u32); 4] = [
    ("particles", &["particles.xr"], lists::PARTICLES_VERSION),
    ("materials", &["gamemtl.xr", "shaders_xrlc.xr", "shaders.xr"], lists::MATERIALS_VERSION),
    ("lightanims", &["lanims.xr"], lists::LIGHTANIMS_VERSION),
    ("sound_envs", &["senvironment.xr"], lists::SOUND_ENVS_VERSION),
];

/// What tells that a list's source changed: the stamps of its files and its version.
fn list_stamp(files: &GameFiles, stamps: &Stamps, paths: &[&str], version: u32) -> String {
    let mut parts: Vec<(String, String)> = paths
        .iter()
        .map(|p| ((*p).to_owned(), files.get(p).map_or_else(|| "missing".to_owned(), |f| stamps.of_file(f))))
        .collect();
    parts.push(("version".to_owned(), version.to_string()));
    join_stamps(&parts)
}

/// The same of the spawn sections: they are the whole config tree's, made again when any
/// config changed.
fn sections_stamp(files: &GameFiles, stamps: &Stamps) -> String {
    let mut configs: Vec<(String, String)> = files
        .iter()
        .filter(|f| f.path.starts_with("configs/") && f.path.ends_with(".ltx"))
        .map(|f| (f.path.to_owned(), stamps.of_file(f)))
        .collect();
    configs.sort();
    configs.push(("version".to_owned(), lists::SECTIONS_VERSION.to_string()));
    // FNV-1a over the stamps: thousands of files told by one word
    format!(
        "{:016x}",
        join_stamps(&configs)
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
    )
}

/// The lists of the game that are one file each, made again when their source changed.
fn sync_lists(
    files: &GameFiles,
    stamps: &Stamps,
    layout: &Layout,
    state: &mut State,
    opts: &Options,
) -> Result<Vec<String>, String> {
    let read = |path: &str| files.get(path).and_then(|f| files.read_ref(f).ok());
    let mut made = Vec::new();
    let mut list =
        |name: &str, paths: &[&str], version: u32, make: &dyn Fn() -> Option<Value>| -> Result<(), String> {
            let stamp = list_stamp(files, stamps, paths, version);
            let key = format!("list:{name}");
            let file = layout.list(name);
            if !opts.rebuild && state.groups.get(&key) == Some(&stamp) && file.is_file() {
                return Ok(());
            }
            let Some(value) = make() else {
                // the game holds none of it: no list
                let _ = std::fs::remove_file(&file);
                state.groups.insert(key, stamp);
                return Ok(());
            };
            let text = lua::rows_file(&card_comment(files, &paths.join(", ")), &value);
            write_atomic(&file, text.as_bytes())
                .map_err(|e| format!("{name}.lua could not be written: {e}"))?;
            state.groups.insert(key, stamp);
            made.push(name.to_owned());
            Ok(())
        };
    let [particles, materials, lightanims, sound_envs] = LISTS;
    list(particles.0, particles.1, particles.2, &|| {
        let lib = gz_fmt_xr::Particles::read(&read("particles.xr")?).ok()?;
        Some(lists::particles(&lib))
    })?;
    list(materials.0, materials.1, materials.2, &|| {
        let mtl = read("gamemtl.xr").and_then(|b| gz_fmt_xr::GameMaterials::read(&b).ok());
        let xrlc = read("shaders_xrlc.xr").and_then(|b| gz_fmt_xr::ShadersXrlc::read(&b).ok());
        let shaders = read("shaders.xr").and_then(|b| gz_fmt_xr::Shaders::read(&b).ok());
        if mtl.is_none() && xrlc.is_none() && shaders.is_none() {
            return None;
        }
        Some(lists::materials(mtl.as_ref(), xrlc.as_ref(), shaders.as_ref()))
    })?;
    list(lightanims.0, lightanims.1, lightanims.2, &|| {
        Some(lists::lightanims(&gz_fmt_xr::LightAnims::read(&read("lanims.xr")?).ok()?))
    })?;
    list(sound_envs.0, sound_envs.1, sound_envs.2, &|| {
        Some(lists::sound_envs(&gz_fmt_xr::SoundEnvironments::read(&read("senvironment.xr")?).ok()?))
    })?;
    let stamp = sections_stamp(files, stamps);
    let key = "list:sections".to_owned();
    let file = layout.list("sections");
    if opts.rebuild || state.groups.get(&key) != Some(&stamp) || !file.is_file() {
        match gz_game::configs::load_tree(files, gz_game::SYSTEM_LTX, None, "the game has no system.ltx") {
            Ok(tree) => {
                let text = lua::rows_file(
                    &card_comment(files, "the configs: system.ltx and what it includes"),
                    &lists::sections(&tree.ltx),
                );
                write_atomic(&file, text.as_bytes())
                    .map_err(|e| format!("sections.lua could not be written: {e}"))?;
                made.push("sections".to_owned());
            }
            Err(_) => {
                let _ = std::fs::remove_file(&file);
            }
        }
        state.groups.insert(key, stamp);
    }
    Ok(made)
}

/// `library.lua`: what the library holds, for a reader.
#[must_use]
pub fn summary(files: &GameFiles, state: &State, report: &Report) -> Value {
    let counts: serde_json::Map<String, Value> =
        FILE_KINDS.iter().map(|k| (k.catalog.to_owned(), json!(state.count(k.name)))).collect();
    let mut problems = Vec::new();
    for (kind, r) in &report.kinds {
        for (file, why) in &r.problems {
            problems.push(json!({ "kind": kind, "file": file, "problem": why }));
        }
    }
    json!({
        "library": 1,
        "game": files.root().to_string_lossy(),
        "complete": !report.stopped,
        "counts": counts,
        "lists": state.groups.keys().filter_map(|k| k.strip_prefix("list:")).collect::<Vec<_>>(),
        "problems": problems,
    })
}

/// Why the library at `layout` is behind the game of `files`: the first thing that differs -
/// a file of a kind, the source of a list, the files of a level - and what differs in it; `None`
/// when it is current. Blocking: the state is read and every file of the game's index looked at.
#[must_use]
pub fn stale_reason(files: &GameFiles, layout: &Layout) -> Option<String> {
    let state = State::read(&layout.state());
    let stamps = Stamps::of(files);
    stale_kind(files, &stamps, &state)
        .or_else(|| {
            LISTS
                .iter()
                .map(|(name, paths, version)| (*name, list_stamp(files, &stamps, paths, *version)))
                .chain(std::iter::once(("sections", sections_stamp(files, &stamps))))
                .find(|(name, stamp)| state.groups.get(&format!("list:{name}")) != Some(stamp))
                .map(|(name, _)| format!("what the list of {name} is made from changed"))
        })
        .or_else(|| {
            crate::levels::stale_levels(files, &stamps, layout, false)
                .first()
                .map(|(level, _)| format!("the level {level} is new or changed"))
        })
}

/// The first file kind that differs from the game, and what differs in it.
fn stale_kind(files: &GameFiles, stamps: &Stamps, state: &State) -> Option<String> {
    FILE_KINDS.iter().find_map(|kind| {
        if state.versions.get(kind.name) != Some(&kind.version) {
            return Some(format!("the {} were described by another version of the editor", kind.title));
        }
        let held = state.files.get(kind.name);
        let ext = format!(".{}", kind.ext);
        let mut count = 0usize;
        let changed = files
            .iter()
            .filter(|f| f.path.starts_with(kind.under) && f.path.ends_with(&ext))
            .find(|f| {
                count += 1;
                held.and_then(|h| h.get(f.path)).map(|e| e.stamp.as_str())
                    != Some(stamps.of_file(*f).as_str())
            })
            .map(|f| f.path.to_owned());
        if let Some(path) = changed {
            return Some(format!("{} is new or changed", path));
        }
        let had = held.map_or(0, FxHashMap::len);
        (had != count).then(|| format!("{} of the {} are gone", had.saturating_sub(count), kind.title))
    })
}

/// Whether the library at `layout` is behind the game of `files`. Blocking.
#[must_use]
pub fn is_stale(files: &GameFiles, layout: &Layout) -> bool {
    stale_reason(files, layout).is_some()
}

/// Brings the library of `files` at `layout` up to date. Blocking; runs its parallel work on
/// the rayon pool it is called in.
pub fn sync(files: &GameFiles, layout: &Layout, opts: &Options, p: &dyn Progress) -> Result<Report, String> {
    let started = Instant::now();
    std::fs::create_dir_all(&layout.root)
        .map_err(|e| format!("the library's folder could not be made: {e}"))?;
    let mut state = State::read(&layout.state());
    let stamps = Stamps::of(files);
    let mut report = Report::default();
    for kind in &FILE_KINDS {
        if !opts.takes(kind.catalog) {
            continue;
        }
        if p.stopped() {
            report.stopped = true;
            break;
        }
        p.stage(&format!("Reading the game's {}", kind.title));
        let r = sync_kind(kind, files, &stamps, layout, &mut state, opts, p)?;
        report.kinds.insert(kind.catalog.to_owned(), r);
        write_atomic(&layout.state(), state.to_text().as_bytes())
            .map_err(|e| format!("the library's state could not be written: {e}"))?;
    }
    if !report.stopped && opts.takes("lists") && opts.only.is_none() {
        p.stage("Reading the game's lists");
        report.lists = sync_lists(files, &stamps, layout, &mut state, opts)?;
    }
    report.stopped = report.stopped || p.stopped();
    report.seconds = started.elapsed().as_secs_f32();
    write_atomic(&layout.state(), state.to_text().as_bytes())
        .map_err(|e| format!("the library's state could not be written: {e}"))?;
    let text = lua::file(&card_comment(files, "every file"), &summary(files, &state, &report));
    write_atomic(&layout.summary(), text.as_bytes())
        .map_err(|e| format!("library.lua could not be written: {e}"))?;
    Ok(report)
}
