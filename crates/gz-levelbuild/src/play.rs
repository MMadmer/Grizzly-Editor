//! Play's game root (D1): the linked game run on the baked scene without a file of its install
//! changed. Which way is the engine's to say, read from its version resource before anything is
//! written ([`PlayMode`], `gz_game::version::engine_abilities`), never found out by trying:
//! - **in place** - an engine that says `fsltx-in-place` (Dead Air Refined from the build that
//!   added it) is started where it is installed, with `-fsltx <root>\fsgame.ltx`: on another fs
//!   config it writes nothing into its own folder (its content records go to the config's app
//!   data, a relative root of the config is the config's folder, no update check) and takes the
//!   archives from that config's `$arch_dir$`, which the root's `fsgame.ltx` names by the game's
//!   own path. The root holds the marker, `fsgame.ltx`, the copy of the loose `gamedata` with the
//!   level and `editor.spawn` staged into it, an `appdata` of its own and an empty `modules` - no
//!   program and no link;
//! - **copied** - an older engine, which rewrites `.dead-air-x64\content-state.txt` beside itself
//!   and starts a level only on that folder's `database`, runs from copies of the engine and its
//!   libraries in the root (copies, not hard links: a link would share a file of the install with
//!   whatever the root's removal does to it), beside a junction to the archives.
//!
//! In both the `appdata` is the root's (the player's settings copied the first time, no save, no
//! module). The game's own QA roots are built the same way.
//!
//! What may be a root is [`crate::play_root::check_root`]'s: it is asked before anything else
//! here, and a root is made only in a missing or empty folder, which then carries the marker
//! first. Play removes or overwrites an entry of the root's top only when the marker it read at
//! the check listed it or Play wrote it in this very run: each name is claimed - its line
//! appended to the marker - before it is written, and a name found there that is neither is a
//! refusal (a root being made, missing or empty at the check, has nothing removed). What Play
//! writes into its root, and the only things it clears there:
//! - the marker (`play_root.txt`), `program = ` and `entry = ` lines - a program it listed and no
//!   longer copies is removed (all of them once the engine runs in place);
//! - `fsgame.ltx`, the game's, overwritten (in place: its `$arch_dir$` names the game's archives);
//! - `database`, the junction to the game's archives of a copied engine (made once; unlinked,
//!   never followed, wherever a root is cleared - and when the engine runs in place);
//! - `gamedata\`, the game's loose gamedata copied anew every time - the old copy removed (a link
//!   in it unlinked) only when the marker listed it and no `project.ltx` / `mod.ltx` lies inside
//!   it, walked again right before -, and the staged level in it;
//! - `appdata\` (logs, saves, screenshots, the player's `user.ltx` and the caches, seeded once and
//!   never cleared; in place the game keeps its content records there too), `modules\` (empty),
//!   `.dead-air-x64\` of a copied engine (the content files copied, the latch removed; removed
//!   when the engine runs in place).
//!
//! Which programs a copied engine takes: the engine, every library the game's own list of its runtime names
//! (`.dead-air-x64\runtime-files.txt`, kept by its installer and updater; without it every library
//! of the game's folder), and every library of the game's folder any of them imports, ordinary or
//! delay-loaded (read from their import tables, [`crate::pe`]) - so a game build that adds a
//! library to its list, or links one it forgot to list, still starts. Never another executable:
//! the updater, the uninstaller and the tools stay where they are.
//!
//! The depth of a root is what the engine takes ([`path_limit`]): a `long-paths` engine 519
//! characters where Windows long paths are on (else `MAX_PATH`, 259), an older one 255
//! (`CLocatorAPI::Register` copied every path it indexed into a `string256`).
//!
//! The editor's C++ Play ran IXRay's `Xr3DA.exe`, which read `level.ai.temp` under `-editor_scene`;
//! Dead Air Refined's engine reads `level.ai`, so the staged level folder names it so.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use gz_project::DeleteContext;

use crate::play_root::{self, MARKER, RootState};

/// What of a game folder is its program: the engines a root is looked into for (the linked
/// game's own first), the extensions of its libraries and the game's list of its runtime.
#[derive(Clone, Copy, Debug)]
pub struct GamePrograms {
    pub engines: &'static [&'static str],
    pub libraries: &'static [&'static str],
    /// Relative to the game's folder: one file name of the runtime per line.
    pub runtime_list: Option<&'static str>,
}

/// Dead Air Refined's.
pub const LINKED_GAME: GamePrograms = GamePrograms {
    engines: &["xrEngine.exe", "Xr3DA.exe"],
    libraries: &["dll"],
    runtime_list: Some(".dead-air-x64\\runtime-files.txt"),
};

/// The engine's word for "started on another fs config, I write nothing into my folder and take
/// the archives from that config's `$arch_dir$`".
pub const IN_PLACE: &str = "fsltx-in-place";
/// The engine's word for "long-path aware, my paths take [`gz_game::version::LONG_PATH_LIMIT`]".
pub const LONG_PATHS: &str = "long-paths";

/// How Play runs the linked game.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayMode {
    /// The installed engine, where it is, on the root's `fsgame.ltx`.
    InPlace,
    /// Copies of the engine and its libraries in the root, beside a junction to the archives.
    Copied,
}

impl PlayMode {
    /// As the answers and the log name it.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::InPlace => "in_place",
            Self::Copied => "copied",
        }
    }
}

/// What an engine says it can, from its version resource: nothing is started to find out.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ability {
    pub in_place: bool,
    pub long_paths: bool,
}

/// [`Ability`] of the engine at `engine`; an engine that cannot be read says nothing.
#[must_use]
pub fn ability_of(engine: &Path) -> Ability {
    let words = fs::read(engine).map(|b| gz_game::version::engine_abilities(&b)).unwrap_or_default();
    let has = |w: &str| words.iter().any(|x| x == w);
    Ability { in_place: has(IN_PLACE), long_paths: has(LONG_PATHS) }
}

/// The longest path the game takes. `long_paths_on` asks Windows, and only for an engine that
/// is long-path aware.
#[must_use]
pub fn path_limit(ability: Ability, long_paths_on: impl FnOnce() -> bool) -> usize {
    match (ability.long_paths, ability.long_paths && long_paths_on()) {
        (true, true) => gz_game::version::LONG_PATH_LIMIT,
        (true, false) => WINDOWS_PATH_LIMIT,
        (false, _) => GAME_PATH_LIMIT,
    }
}

/// Whether Windows takes paths over `MAX_PATH` from a long-path aware program
/// (`HKLM\SYSTEM\CurrentControlSet\Control\FileSystem` `LongPathsEnabled`).
#[must_use]
pub fn windows_long_paths() -> bool {
    std::process::Command::new("reg.exe")
        .args(["query", r"HKLM\SYSTEM\CurrentControlSet\Control\FileSystem", "/v", "LongPathsEnabled"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .is_ok_and(|o| {
            o.status.success() && String::from_utf8_lossy(&o.stdout).split_whitespace().last() == Some("0x1")
        })
}

/// The game's `fsgame.ltx` for a root its engine runs on in place: `$arch_dir$`, rooted at
/// `$fs_root$` in the game's, is rooted at the game's folder instead, so the archives are read
/// where they are installed and the root needs no link to them. Every other line stays.
#[must_use]
pub fn fsgame_in_place(text: &str, game: &Path) -> String {
    let game = game.to_string_lossy();
    let game = game.trim_end_matches('\\');
    let mut out = String::with_capacity(text.len() + game.len());
    for line in text.split_inclusive('\n') {
        let (id, value) = line.split_once('=').unwrap_or((line, ""));
        let mut items: Vec<String> = value.split('|').map(str::to_owned).collect();
        if id.trim().eq_ignore_ascii_case("$arch_dir$")
            && items.len() >= 3
            && items[2].trim().eq_ignore_ascii_case("$fs_root$")
        {
            items[2] = format!(" {game}\\");
            out.push_str(id);
            out.push('=');
            out.push_str(&items.join("|"));
        } else {
            out.push_str(line);
        }
    }
    out
}
/// What the game's content service reads beside the executable; a stale incomplete latch would
/// keep it at the main menu.
const CONTENT_DIR: &str = ".dead-air-x64";
const CONTENT_FILES: [&str; 2] = ["content-manifest.txt", "content-state.txt"];
const CONTENT_LATCH: &str = "content-incomplete.txt";
/// Caches of the player's `appdata` seeded once, so a Play does not compile every shader again.
const CACHES: [&str; 4] = ["shaders_cache", "shaders_cache_grizzly", "cdb_cache", "render-hardware.cache"];

/// A game root ready to run.
#[derive(Clone, Debug)]
pub struct PlayRoot {
    pub root: PathBuf,
    /// The engine to start: the game's own in place, the root's copy otherwise.
    pub exe: PathBuf,
    pub mode: PlayMode,
    /// The longest path the game takes ([`path_limit`]).
    pub path_limit: usize,
    /// The engine and the libraries copied, by name (none in place).
    pub programs: Vec<String>,
    pub program_bytes: u64,
    /// What the root was before: `missing`, `empty` or `play_root`.
    pub was: &'static str,
    /// The entries of the old gamedata walked for a project before it was cleared.
    pub walked: u32,
    /// How long that walk took, in milliseconds.
    pub walk_ms: u64,
}

impl PlayRoot {
    /// What goes in front of the caller's parameters: the root's `fsgame.ltx` - by its full path
    /// for the engine in place (which takes a quoted path), by name for a copy, which runs in the
    /// root and reads a path up to its first space.
    #[must_use]
    pub fn fsltx_args(&self) -> Vec<String> {
        let config = match self.mode {
            PlayMode::InPlace => self.root.join("fsgame.ltx").to_string_lossy().into_owned(),
            PlayMode::Copied => "fsgame.ltx".to_owned(),
        };
        vec!["-fsltx".to_owned(), config]
    }
}

/// What a Play prepares: the linked game, the root, what the checks compare with, the longest
/// path of the game's archives (relative to its gamedata) and the level it stages, if any.
pub struct PlayInput<'a> {
    pub game: &'a Path,
    pub root: &'a Path,
    pub context: &'a DeleteContext,
    pub archived: &'a str,
    /// The level's name and the folder it was built into.
    pub level: Option<(&'a str, &'a Path)>,
}

/// The engine of a game folder.
#[must_use]
pub fn engine_of(game: &Path, programs: &GamePrograms) -> Option<PathBuf> {
    programs.engines.iter().map(|e| game.join(e)).find(|p| p.is_file())
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.len() == y.len() && x.modified().ok() == y.modified().ok(),
        _ => false,
    }
}

/// `src` copied to `dst` unless the copy there is already of the same size and time.
fn copy_changed(src: &Path, dst: &Path) -> Result<(), String> {
    if same_file(src, dst) {
        return Ok(());
    }
    fs::copy(src, dst).map(|_| ()).map_err(|e| format!("{} could not be copied: {e}", src.display()))
}

fn copy_tree(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("{} could not be made: {e}", dst.display()))?;
    let entries = fs::read_dir(src).map_err(|e| format!("{} could not be read: {e}", src.display()))?;
    for e in entries.flatten() {
        let Ok(t) = e.file_type() else { continue };
        let to = dst.join(e.file_name());
        if t.is_dir() {
            copy_tree(&e.path(), &to)?;
        } else if t.is_file() {
            fs::copy(e.path(), &to)
                .map_err(|err| format!("{} could not be copied: {err}", e.path().display()))?;
        }
    }
    Ok(())
}

/// A folder removed for good; a link in it goes as a link and is never followed.
fn remove_tree(dir: &Path) {
    let Ok(meta) = fs::symlink_metadata(dir) else { return };
    if meta.file_type().is_symlink() || is_junction(dir) {
        let _ = fs::remove_dir(dir).or_else(|_| fs::remove_file(dir));
        return;
    }
    gz_build::install::remove_tree(dir);
}

/// Whether `p` is a reparse point (a junction, a link): its target is not this tree's.
fn is_junction(p: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // FILE_ATTRIBUTE_REPARSE_POINT
        fs::symlink_metadata(p).is_ok_and(|m| m.file_attributes() & 0x400 != 0)
    }
    #[cfg(not(windows))]
    {
        let _ = p;
        false
    }
}

/// `root\database` as a junction to the game's archives (a junction needs no privilege).
fn link_archives(game: &Path, root: &Path) -> Result<(), String> {
    let link = root.join("database");
    let target = game.join("database");
    if !target.is_dir() {
        return Ok(());
    }
    if is_junction(&link) {
        return Ok(());
    }
    let made = std::process::Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if made {
        Ok(())
    } else {
        Err(format!("the link to the game's archives could not be made in {}", root.display()))
    }
}

/// The files of the game folder's top, by lower-case name; links left out.
fn top_files(game: &Path) -> Result<BTreeMap<String, String>, String> {
    let entries = fs::read_dir(game).map_err(|e| format!("{} could not be read: {e}", game.display()))?;
    Ok(entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| e.file_name().to_str().map(|n| (n.to_ascii_lowercase(), n.to_owned())))
        .collect())
}

fn has_extension(name: &str, extensions: &[&str]) -> bool {
    name.rsplit_once('.').is_some_and(|(_, x)| extensions.iter().any(|e| e.eq_ignore_ascii_case(x)))
}

/// `start` and every library of `top` that one of them imports, again and again.
fn close_over(
    mut set: std::collections::BTreeSet<String>,
    top: &BTreeMap<String, String>,
    libraries: &[&str],
    mut imports: impl FnMut(&str) -> Vec<String>,
) -> std::collections::BTreeSet<String> {
    let mut queue: Vec<String> = set.iter().cloned().collect();
    while let Some(n) = queue.pop() {
        let Some(name) = top.get(&n) else { continue };
        for i in imports(name) {
            let l = i.to_ascii_lowercase();
            if top.contains_key(&l) && has_extension(&l, libraries) && set.insert(l.clone()) {
                queue.push(l);
            }
        }
    }
    set
}

/// The programs Play copies from `game`: the engine, the runtime's libraries, what they import.
fn programs_of(game: &Path, programs: &GamePrograms, engine: &Path) -> Result<Vec<String>, String> {
    let top = top_files(game)?;
    let mut set = std::collections::BTreeSet::new();
    if let Some(e) = engine.file_name().and_then(|n| n.to_str()) {
        set.insert(e.to_ascii_lowercase());
    }
    let listed = programs.runtime_list.and_then(|l| fs::read(game.join(l)).ok());
    match listed {
        Some(text) => {
            for line in String::from_utf8_lossy(&text).lines() {
                let n = line.trim().to_ascii_lowercase();
                if play_root::plain_name(&n) && top.contains_key(&n) && has_extension(&n, programs.libraries)
                {
                    set.insert(n);
                }
            }
        }
        None => set.extend(top.keys().filter(|n| has_extension(n, programs.libraries)).cloned()),
    }
    let set = close_over(set, &top, programs.libraries, |name| {
        fs::File::open(game.join(name)).map(|mut f| crate::pe::imports(&mut f)).unwrap_or_default()
    });
    Ok(set.iter().filter_map(|l| top.get(l).cloned()).collect())
}

/// Makes `input.root` a game root of the linked game at `input.game`, ready for
/// [`stage_level`]. Every refusal comes before the disk is changed. Blocking.
pub fn prepare(input: &PlayInput<'_>) -> Result<PlayRoot, String> {
    prepare_with(input, &LINKED_GAME)
}

/// [`prepare`] for a game whose programs are `programs`.
pub fn prepare_with(input: &PlayInput<'_>, programs: &GamePrograms) -> Result<PlayRoot, String> {
    let state = play_root::check_root(input.context, input.root)?;
    prepare_checked(input, programs, &state)
}

/// The entries of a root's top Play writes besides its programs and the marker: for a copied
/// engine, and for the engine in place.
const ENTRIES: [&str; 6] = ["fsgame.ltx", "database", "gamedata", "appdata", "modules", CONTENT_DIR];
const ENTRIES_IN_PLACE: [&str; 4] = ["fsgame.ltx", "gamedata", "appdata", "modules"];
/// Where an engine in place keeps its content records: the root's app data.
const CONTENT_IN_PLACE: &str = "appdata\\content\\content-state.txt";

/// What Play may remove or overwrite in a root: what its marker listed when the root was checked,
/// and what Play wrote in this very run. A name is claimed - and its line appended to the marker -
/// before it is written; a name that is there and neither is a refusal, and nothing of it changes.
struct Claims<'a> {
    root: &'a Path,
    ours: Vec<String>,
    marker: Option<fs::File>,
}

impl<'a> Claims<'a> {
    fn open(root: &'a Path, state: &RootState) -> Result<Self, String> {
        let at = root.join(MARKER);
        let (ours, marker) = match state {
            RootState::Marked { programs, entries } => {
                let marker = fs::OpenOptions::new()
                    .append(true)
                    .open(&at)
                    .map_err(|e| format!("{} cannot be written: {e}", at.display()))?;
                let ours = programs.iter().chain(entries).map(|n| n.to_ascii_lowercase()).collect();
                (ours, marker)
            }
            RootState::Missing | RootState::Empty => {
                fs::create_dir_all(root).map_err(|e| format!("{} could not be made: {e}", root.display()))?;
                // made, never opened: a marker that is there already is not Play's of this run
                let mut marker =
                    fs::OpenOptions::new().write(true).create_new(true).open(&at).map_err(|e| {
                        if e.kind() == std::io::ErrorKind::AlreadyExists {
                            appeared(root, MARKER)
                        } else {
                            format!("{} could not be written: {e}", at.display())
                        }
                    })?;
                marker
                    .write_all(play_root::marker_head().as_bytes())
                    .map_err(|e| format!("{} could not be written: {e}", at.display()))?;
                (Vec::new(), marker)
            }
        };
        let mut ours: Vec<String> = ours;
        ours.push(MARKER.to_ascii_lowercase());
        Ok(Self { root, ours, marker: Some(marker) })
    }

    fn is_ours(&self, name: &str) -> bool {
        self.ours.iter().any(|o| o.eq_ignore_ascii_case(name))
    }

    fn claim(&mut self, name: &str, program: bool) -> Result<(), String> {
        if self.is_ours(name) {
            return Ok(());
        }
        if fs::symlink_metadata(self.root.join(name)).is_ok() {
            return Err(appeared(self.root, name));
        }
        if let Some(m) = self.marker.as_mut() {
            m.write_all(play_root::marker_line(program, name).as_bytes())
                .and_then(|()| m.flush())
                .map_err(|e| format!("{MARKER} could not be written in {}: {e}", self.root.display()))?;
        }
        self.ours.push(name.to_ascii_lowercase());
        Ok(())
    }
}

fn appeared(root: &Path, name: &str) -> String {
    format!(
        "{name} is in the game root {} and Play did not write it: the root's {MARKER} did not list it when Play checked the root, and this run has not written it. Play leaves it as it is; give Play an empty folder.",
        root.display()
    )
}

/// [`prepare_with`] after [`play_root::check_root`] said what the root holds: everything from
/// there on, every removal and overwrite limited to what `state`'s marker listed and what this
/// run writes. Blocking.
fn prepare_checked(
    input: &PlayInput<'_>,
    programs: &GamePrograms,
    state: &RootState,
) -> Result<PlayRoot, String> {
    let (game, root) = (input.game, input.root);
    let engine = engine_of(game, programs).ok_or_else(|| {
        format!("the linked game at {} has no {}", game.display(), programs.engines.join(" or "))
    })?;
    let fsgame = game.join("fsgame.ltx");
    if !fsgame.is_file() {
        return Err(format!("the linked game at {} has no fsgame.ltx", game.display()));
    }
    // the engine says which way it runs; nothing is tried
    let ability = ability_of(&engine);
    let mode = if ability.in_place { PlayMode::InPlace } else { PlayMode::Copied };
    let config = match mode {
        PlayMode::InPlace => {
            let text =
                fs::read(&fsgame).map_err(|e| format!("{} could not be read: {e}", fsgame.display()))?;
            let text = fsgame_in_place(&gz_base::text::cp1251_to_utf8(&text), game);
            let (bytes, lossy) = gz_base::text::utf8_to_cp1251(&text);
            if lossy {
                return Err(format!(
                    "The game's folder {} has a character in its name the game cannot read (it reads paths in the Windows-1251 code page), so its fs config cannot name the game's archives.",
                    game.display()
                ));
            }
            Some(bytes.into_owned())
        }
        PlayMode::Copied => None,
    };
    let copied = match mode {
        PlayMode::InPlace => Vec::new(),
        PlayMode::Copied => programs_of(game, programs, &engine)?,
    };
    let limit = path_limit(ability, windows_long_paths);
    check_path_lengths(root, &planned_longest(input, state, &copied, mode), limit)?;
    let (old_programs, old_entries) = match state {
        RootState::Marked { programs, entries } => (programs.clone(), entries.clone()),
        RootState::Missing | RootState::Empty => (Vec::new(), Vec::new()),
    };
    let listed = |name: &str| old_entries.iter().any(|e| e.eq_ignore_ascii_case(name));

    // from here on the disk changes: the marker first, then each entry claimed before it is written
    let mut claims = Claims::open(root, state)?;
    let mut bytes = 0;
    for name in &copied {
        claims.claim(name, true)?;
        let src = game.join(name);
        copy_changed(&src, &root.join(name))?;
        bytes += fs::metadata(&src).map(|m| m.len()).unwrap_or(0);
    }
    // a program the marker listed that the game no longer has: Play's own, so it goes
    let stale: Vec<&String> =
        old_programs.iter().filter(|o| !copied.iter().any(|c| c.eq_ignore_ascii_case(o))).collect();
    for name in &stale {
        let p = root.join(name);
        if fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()) {
            fs::remove_file(&p).map_err(|e| format!("{} could not be removed: {e}", p.display()))?;
        }
    }
    claims.claim("fsgame.ltx", false)?;
    match &config {
        Some(text) => fs::write(root.join("fsgame.ltx"), text),
        None => fs::copy(&fsgame, root.join("fsgame.ltx")).map(|_| ()),
    }
    .map_err(|e| format!("fsgame.ltx could not be written in {}: {e}", root.display()))?;
    match mode {
        // a junction already there is kept; anything else of that name was refused by the claim
        PlayMode::Copied => {
            claims.claim("database", false)?;
            link_archives(game, root)?;
        }
        // the archives are read where they are: a link a copied engine needed goes, as a link
        PlayMode::InPlace => {
            let link = root.join("database");
            if listed("database") && is_junction(&link) {
                remove_tree(&link);
            }
        }
    }
    // the loose gamedata as the game has it now; the old copy goes only from a root whose marker
    // listed it at the check, and only when no project lies inside it
    claims.claim("gamedata", false)?;
    let gamedata = root.join("gamedata");
    let (mut walked, mut walk_ms) = (0, 0);
    if listed("gamedata") {
        let t = std::time::Instant::now();
        walked = play_root::nothing_to_lose(root, &old_entries)?;
        walk_ms = u64::try_from(t.elapsed().as_millis()).unwrap_or(u64::MAX);
        remove_tree(&gamedata);
    }
    if game.join("gamedata").is_dir() {
        copy_tree(&game.join("gamedata"), &gamedata)?;
    }
    // an appdata of its own: the player's settings, the caches once, no save; nothing in it is
    // ever removed or overwritten
    claims.claim("appdata", false)?;
    let appdata = root.join("appdata");
    for d in ["logs", "savedgames", "screenshots"] {
        let _ = fs::create_dir_all(appdata.join(d));
    }
    let user = game.join("appdata").join("user.ltx");
    if user.is_file() && !appdata.join("user.ltx").exists() {
        fs::copy(&user, appdata.join("user.ltx"))
            .map_err(|e| format!("user.ltx could not be copied: {e}"))?;
    }
    for c in CACHES {
        let (src, dst) = (game.join("appdata").join(c), appdata.join(c));
        if dst.exists() {
            continue;
        }
        if src.is_dir() {
            copy_tree(&src, &dst)?;
        } else if src.is_file() {
            let _ = fs::copy(&src, &dst);
        }
    }
    // no module of the player's: the game root has a modules folder of its own, empty
    claims.claim("modules", false)?;
    let _ = fs::create_dir_all(root.join("modules"));
    let content = root.join(CONTENT_DIR);
    match mode {
        PlayMode::Copied => {
            claims.claim(CONTENT_DIR, false)?;
            let _ = fs::create_dir_all(&content);
            for f in CONTENT_FILES {
                let src = game.join(CONTENT_DIR).join(f);
                if src.is_file() {
                    let _ = fs::copy(&src, content.join(f));
                }
            }
            let _ = fs::remove_file(content.join(CONTENT_LATCH));
        }
        // the engine in place keeps its content records in the root's app data itself
        PlayMode::InPlace => {
            if listed(CONTENT_DIR) {
                remove_tree(&content);
            }
        }
    }
    // the marker as the root is now: the programs copied, the entries written
    drop(claims.marker.take());
    let written: &[&str] = match mode {
        PlayMode::Copied => &ENTRIES,
        PlayMode::InPlace => &ENTRIES_IN_PLACE,
    };
    let entries: Vec<String> = written.iter().map(|e| (*e).to_owned()).collect();
    fs::write(root.join(MARKER), play_root::marker_text(&copied, &entries))
        .map_err(|e| format!("{MARKER} could not be written in {}: {e}", root.display()))?;
    let exe = match mode {
        PlayMode::InPlace => engine,
        PlayMode::Copied => root.join(engine.file_name().unwrap_or_default()),
    };
    Ok(PlayRoot {
        root: root.to_path_buf(),
        exe,
        mode,
        path_limit: limit,
        programs: copied,
        program_bytes: bytes,
        was: state.name(),
        walked,
        walk_ms,
    })
}

/// The longest file path an engine that is not long-path aware takes: its `CLocatorAPI::Register`
/// copied every path it indexes into a `string256`, and a longer one stopped the game at its start
/// (a breakpoint in xrCore, before its log opens - what a root inside a project folder, or a deep
/// temp folder, did).
pub const GAME_PATH_LIMIT: usize = 255;
/// `MAX_PATH` less the terminator: what Windows takes from a long-path aware engine while its
/// long paths are off.
pub const WINDOWS_PATH_LIMIT: usize = 259;

/// The longest path below `dir` (relative to the root, backslashes, after `rel`), links not
/// followed.
fn longest_below(dir: &Path, rel: &str, longest: &mut String) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = if rel.is_empty() { name } else { format!("{rel}\\{name}") };
        let Ok(t) = e.file_type() else { continue };
        if t.is_symlink() || is_junction(&e.path()) {
            continue;
        }
        if t.is_dir() {
            longest_below(&e.path(), &path, longest);
        } else {
            keep_longer(longest, path);
        }
    }
}

fn keep_longer(longest: &mut String, path: String) {
    if path.chars().count() > longest.chars().count() {
        *longest = path;
    }
}

/// The longest path the root will hold once Play has prepared it and staged its level: what
/// it copies of the game, what it stages, what the root keeps (its appdata), the archives.
fn planned_longest(input: &PlayInput<'_>, state: &RootState, programs: &[String], mode: PlayMode) -> String {
    let mut longest = String::new();
    longest_below(&input.game.join("gamedata"), "gamedata", &mut longest);
    for c in CACHES {
        let src = input.game.join("appdata").join(c);
        if src.is_dir() {
            longest_below(&src, &format!("appdata\\{c}"), &mut longest);
        } else if src.is_file() {
            keep_longer(&mut longest, format!("appdata\\{c}"));
        }
    }
    if matches!(state, RootState::Marked { .. })
        && let Ok(entries) = fs::read_dir(input.root)
    {
        // what the root keeps: gamedata is copied anew and the archives' link is not followed
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.eq_ignore_ascii_case("gamedata") || name.eq_ignore_ascii_case("database") {
                continue;
            }
            if e.file_type().is_ok_and(|t| t.is_dir()) && !is_junction(&e.path()) {
                longest_below(&e.path(), &name, &mut longest);
            } else {
                keep_longer(&mut longest, name);
            }
        }
    }
    if let Some((level, dir)) = input.level {
        let rel = format!("gamedata\\levels\\{level}");
        longest_below(dir, &rel, &mut longest);
        keep_longer(&mut longest, format!("{rel}\\level.ai"));
        keep_longer(&mut longest, "gamedata\\spawns\\editor.spawn".to_owned());
    }
    if !input.archived.is_empty() {
        keep_longer(&mut longest, format!("gamedata\\{}", input.archived.replace('/', "\\")));
    }
    for p in programs {
        keep_longer(&mut longest, p.clone());
    }
    match mode {
        PlayMode::Copied => {
            for f in CONTENT_FILES {
                keep_longer(&mut longest, format!("{CONTENT_DIR}\\{f}"));
            }
        }
        PlayMode::InPlace => keep_longer(&mut longest, CONTENT_IN_PLACE.to_owned()),
    }
    longest
}

/// Refuses a root the game could not start in: `longest`, the longest path it will hold
/// (relative to it), has to fit `limit` ([`path_limit`]) with the root in front.
fn check_path_lengths(root: &Path, longest: &str, limit: usize) -> Result<(), String> {
    let root_text = root.to_string_lossy();
    let total = root_text.trim_end_matches('\\').chars().count() + 1 + longest.chars().count();
    if total > limit {
        return Err(format!(
            "The game root {root_text} is too deep for the game: with {longest} in it a path is {total} characters long, and the game takes {limit} at most. Give Play a shorter root."
        ));
    }
    Ok(())
}

/// The built level (`level_dir`, every file of it) into the root's `gamedata\levels\<level>`,
/// `level.ai` among them, and the spawn as `gamedata\spawns\editor.spawn`. Only into a root
/// Play made (its marker). Blocking.
pub fn stage_level(
    root: &Path,
    level: &str,
    level_dir: &Path,
    level_ai: &[u8],
    spawn: &[u8],
) -> Result<(), String> {
    // only into the gamedata a Play root's marker lists: the copy Play made of the game's
    if !play_root::read_marker(root).is_some_and(|(_, entries)| entries.iter().any(|e| e == "gamedata")) {
        return Err(format!("{} is not a Play root: its {MARKER} does not list gamedata.", root.display()));
    }
    if !play_root::plain_name(level) {
        return Err(format!("{level} is not a level name"));
    }
    let dst = root.join("gamedata").join("levels").join(level);
    remove_tree(&dst);
    copy_tree(level_dir, &dst)?;
    fs::write(dst.join("level.ai"), level_ai).map_err(|e| format!("level.ai could not be staged: {e}"))?;
    let spawns = root.join("gamedata").join("spawns");
    fs::create_dir_all(&spawns).map_err(|e| format!("{} could not be made: {e}", spawns.display()))?;
    fs::write(spawns.join("editor.spawn"), spawn)
        .map_err(|e| format!("editor.spawn could not be staged: {e}"))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    // no test writes a file named like a program: the game's programs are harmless names here
    const PROGRAMS: GamePrograms =
        GamePrograms { engines: &["engine.bin"], libraries: &["lib"], runtime_list: Some("runtime.txt") };

    fn game(base: &Path) -> PathBuf {
        let game = base.join("game");
        fs::create_dir_all(game.join("gamedata").join("configs")).unwrap();
        fs::create_dir_all(game.join("database")).unwrap();
        fs::create_dir_all(game.join("appdata").join("savedgames")).unwrap();
        fs::create_dir_all(game.join("modules").join("someone")).unwrap();
        fs::write(game.join("engine.bin"), b"engine").unwrap();
        fs::write(game.join("core.lib"), b"library").unwrap();
        fs::write(game.join("unused.lib"), b"an old library").unwrap();
        fs::write(game.join("updater.bin"), b"another program").unwrap();
        fs::write(game.join("runtime.txt"), b"engine.bin\r\ncore.lib\r\n..\\outside.lib\r\n").unwrap();
        fs::write(game.join("readme.txt"), b"not copied").unwrap();
        fs::write(game.join("fsgame.ltx"), b"$fs_root$").unwrap();
        fs::write(game.join("gamedata").join("configs").join("system.ltx"), b"[x]").unwrap();
        fs::write(game.join("database").join("a.db0"), b"db").unwrap();
        fs::write(game.join("appdata").join("user.ltx"), b"vid_mode 800x600").unwrap();
        fs::write(game.join("appdata").join("savedgames").join("s.scop"), b"save").unwrap();
        game
    }

    fn input<'a>(game: &'a Path, root: &'a Path, cx: &'a DeleteContext) -> PlayInput<'a> {
        PlayInput { game, root, context: cx, archived: "", level: None }
    }

    #[test]
    fn a_play_root_mirrors_the_game_and_changes_nothing_of_it() {
        let base = tempfile::tempdir().unwrap();
        let game = game(base.path());
        let cx = DeleteContext::default();
        let root = base.path().join("play");
        let p = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap();
        assert_eq!(p.exe, root.join("engine.bin"));
        assert_eq!(p.was, "missing");
        assert_eq!(p.programs, ["core.lib", "engine.bin"], "the runtime's list, not every library");
        assert_eq!(fs::read(root.join("core.lib")).unwrap(), b"library");
        assert!(!root.join("unused.lib").exists() && !root.join("updater.bin").exists());
        assert!(!root.join("readme.txt").exists(), "only the programs are copied");
        let marker = fs::read_to_string(root.join(MARKER)).unwrap();
        assert!(marker.contains("program = core.lib") && marker.contains("program = engine.bin"), "{marker}");
        assert!(root.join("gamedata").join("configs").join("system.ltx").is_file());
        assert!(is_junction(&root.join("database")) && root.join("database").join("a.db0").is_file());
        assert_eq!(fs::read(root.join("appdata").join("user.ltx")).unwrap(), b"vid_mode 800x600");
        assert!(!root.join("appdata").join("savedgames").join("s.scop").exists(), "no save of the player's");
        assert!(
            root.join("modules").is_dir() && fs::read_dir(root.join("modules")).unwrap().next().is_none()
        );

        let level = base.path().join("built");
        fs::create_dir_all(&level).unwrap();
        fs::write(level.join("level"), b"geometry").unwrap();
        stage_level(&root, "qa_level", &level, b"ai", b"spawn").unwrap();
        let staged = root.join("gamedata").join("levels").join("qa_level");
        assert_eq!(fs::read(staged.join("level")).unwrap(), b"geometry");
        assert_eq!(fs::read(staged.join("level.ai")).unwrap(), b"ai");
        assert_eq!(fs::read(root.join("gamedata").join("spawns").join("editor.spawn")).unwrap(), b"spawn");

        // again, after the game dropped a library and the author left a file in the root: the
        // dropped library goes (Play copied it), the author's file and the root's settings stay
        fs::write(game.join("runtime.txt"), b"engine.bin\r\n").unwrap();
        fs::write(root.join("notes.txt"), b"mine").unwrap();
        fs::write(root.join("appdata").join("user.ltx"), b"vid_mode 1024x768").unwrap();
        let again = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap();
        assert_eq!(again.was, "play_root");
        assert_eq!(again.programs, ["engine.bin"]);
        assert!(!root.join("core.lib").exists() && game.join("core.lib").is_file());
        assert_eq!(fs::read(root.join("notes.txt")).unwrap(), b"mine");
        assert_eq!(fs::read(root.join("appdata").join("user.ltx")).unwrap(), b"vid_mode 1024x768");
        assert!(!game.join("gamedata").join("levels").exists());
        assert!(!root.join("gamedata").join("levels").exists(), "a new Play stages its level anew");
        // the archives' link goes as a link: the game's archives stay
        remove_tree(&root);
        assert!(game.join("database").join("a.db0").is_file());
        assert_eq!(fs::read(game.join("fsgame.ltx")).unwrap(), b"$fs_root$");
    }

    #[test]
    fn without_the_games_list_every_library_and_never_another_program() {
        let base = tempfile::tempdir().unwrap();
        let game = game(base.path());
        fs::remove_file(game.join("runtime.txt")).unwrap();
        let cx = DeleteContext::default();
        let root = base.path().join("play");
        let p = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap();
        assert_eq!(p.programs, ["core.lib", "engine.bin", "unused.lib"]);
        assert!(!root.join("updater.bin").exists());
    }

    #[test]
    fn a_library_one_of_them_imports_is_copied_though_the_list_forgets_it() {
        let top: BTreeMap<String, String> = ["engine.bin", "core.lib", "render.lib", "tool.bin", "other.lib"]
            .iter()
            .map(|n| (n.to_ascii_lowercase(), (*n).to_owned()))
            .collect();
        let start: BTreeSet<String> = ["engine.bin".to_owned()].into();
        let got = close_over(start, &top, &["lib"], |name| match name {
            "engine.bin" => vec!["CORE.lib".to_owned(), "KERNEL32.sys".to_owned(), "tool.bin".to_owned()],
            "core.lib" => vec!["render.lib".to_owned()],
            _ => Vec::new(),
        });
        let got: Vec<&str> = got.iter().map(String::as_str).collect();
        assert_eq!(got, ["core.lib", "engine.bin", "render.lib"], "a program it names is never copied");
    }

    #[test]
    fn every_refusal_comes_before_the_disk_changes() {
        let base = tempfile::tempdir().unwrap();
        let game = game(base.path());
        let cx = DeleteContext::default();
        // a folder of the author's: refused, untouched
        let work = base.path().join("work");
        fs::create_dir_all(work.join("gamedata")).unwrap();
        fs::write(work.join("gamedata").join("precious.txt"), b"the author's work").unwrap();
        fs::write(work.join("fsgame.ltx"), b"the folder's own").unwrap();
        let why = prepare_with(&input(&game, &work, &cx), &PROGRAMS).unwrap_err();
        assert!(why.contains("is not a Play root: choose an empty folder"), "{why}");
        assert_eq!(fs::read(work.join("gamedata").join("precious.txt")).unwrap(), b"the author's work");
        assert_eq!(fs::read(work.join("fsgame.ltx")).unwrap(), b"the folder's own");
        assert_eq!(fs::read_dir(&work).unwrap().count(), 2);
        // a project: refused, untouched
        fs::write(work.join("mod.ltx"), b"[module]").unwrap();
        fs::remove_file(work.join("fsgame.ltx")).unwrap();
        fs::create_dir_all(work.join("empty")).unwrap();
        let why = prepare_with(&input(&game, &work.join("empty"), &cx), &PROGRAMS).unwrap_err();
        assert!(why.contains("it lies inside the project"), "{why}");
        assert_eq!(fs::read_dir(work.join("empty")).unwrap().count(), 0);
        // too deep: refused before the root is made
        let root = base.path().join("deep");
        let room = GAME_PATH_LIMIT - root.to_string_lossy().chars().count() - 1 - "gamedata\\".len();
        let fits = PlayInput { archived: &"a".repeat(room), ..input(&game, &root, &cx) };
        let longest = planned_longest(&fits, &RootState::Missing, &[], PlayMode::Copied);
        assert!(check_path_lengths(&root, &longest, GAME_PATH_LIMIT).is_ok());
        let deep = "a".repeat(room + 1);
        let refused =
            prepare_with(&PlayInput { archived: &deep, ..input(&game, &root, &cx) }, &PROGRAMS).unwrap_err();
        assert!(
            refused.contains("too deep") && refused.contains(&format!("{}", GAME_PATH_LIMIT + 1)),
            "{refused}"
        );
        assert!(!root.exists(), "nothing was made");
        // a level is staged only into a root Play made
        let level = base.path().join("built");
        fs::create_dir_all(&level).unwrap();
        assert!(
            stage_level(&work, "qa_level", &level, b"ai", b"spawn").unwrap_err().contains("not a Play root")
        );
        assert!(!work.join("gamedata").join("levels").exists());
    }

    #[test]
    fn a_root_that_gained_entries_after_its_check_loses_none_of_them() {
        let base = tempfile::tempdir().unwrap();
        let game = game(base.path());
        let cx = DeleteContext::default();
        // an empty folder at the check, a gamedata of someone's by the time Play would clear it
        let root = base.path().join("empty");
        fs::create_dir_all(&root).unwrap();
        let state = play_root::check_root(&cx, &root).unwrap();
        assert_eq!(state, RootState::Empty);
        fs::create_dir_all(root.join("gamedata")).unwrap();
        fs::write(root.join("gamedata").join("precious.txt"), b"the author's work").unwrap();
        let why = prepare_checked(&input(&game, &root, &cx), &PROGRAMS, &state).unwrap_err();
        assert!(
            why.contains("gamedata is in the game root") && why.contains("Play did not write it"),
            "{why}"
        );
        assert_eq!(fs::read(root.join("gamedata").join("precious.txt")).unwrap(), b"the author's work");
        assert_eq!(fs::read_dir(root.join("gamedata")).unwrap().count(), 1);
        // missing at the check, a library and fsgame.ltx of someone's there now: neither overwritten
        for (name, text) in
            [("core.lib", b"someone's library".as_slice()), ("fsgame.ltx", b"someone's".as_slice())]
        {
            let root = base.path().join(format!("missing_{name}"));
            let state = play_root::check_root(&cx, &root).unwrap();
            assert_eq!(state, RootState::Missing);
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join(name), text).unwrap();
            let why = prepare_checked(&input(&game, &root, &cx), &PROGRAMS, &state).unwrap_err();
            assert!(why.contains(&format!("{name} is in the game root")), "{why}");
            assert_eq!(fs::read(root.join(name)).unwrap(), text);
        }
        // a marker that is there by then is no marker of this run
        let root = base.path().join("marker");
        let state = play_root::check_root(&cx, &root).unwrap();
        fs::create_dir_all(root.join("gamedata")).unwrap();
        fs::write(root.join(MARKER), play_root::marker_text(&[], &["gamedata".to_owned()])).unwrap();
        fs::write(root.join("gamedata").join("precious.txt"), b"mine").unwrap();
        assert!(prepare_checked(&input(&game, &root, &cx), &PROGRAMS, &state).unwrap_err().contains(MARKER));
        assert_eq!(fs::read(root.join("gamedata").join("precious.txt")).unwrap(), b"mine");
    }

    #[test]
    fn a_marked_root_loses_only_what_its_marker_lists_and_never_a_project() {
        let base = tempfile::tempdir().unwrap();
        let game = game(base.path());
        let cx = DeleteContext::default();
        let root = base.path().join("play");
        prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap();
        let marker = fs::read_to_string(root.join(MARKER)).unwrap();
        assert!(marker.contains("entry = gamedata") && marker.contains("entry = fsgame.ltx"), "{marker}");
        // a project inside the gamedata Play would clear: refused, at the check and in prepare
        let nested = root.join("gamedata").join("mods").join("my_mod");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("mod.ltx"), b"[module]").unwrap();
        let why = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap_err();
        assert!(why.contains("it holds a project") && why.contains("my_mod"), "{why}");
        // the project put there after the check: prepare walks again before it clears
        let (programs, entries) = play_root::read_marker(&root).unwrap();
        let stale = RootState::Marked { programs, entries };
        let why = prepare_checked(&input(&game, &root, &cx), &PROGRAMS, &stale).unwrap_err();
        assert!(why.contains("it holds a project"), "{why}");
        assert!(nested.join("mod.ltx").is_file());
        fs::remove_dir_all(root.join("gamedata").join("mods")).unwrap();
        // a gamedata the marker does not list stays: the root is refused
        fs::write(root.join(MARKER), play_root::marker_text(&["engine.bin".to_owned()], &[])).unwrap();
        fs::write(root.join("gamedata").join("precious.txt"), b"mine").unwrap();
        let why = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap_err();
        assert!(why.contains("is in the game root") && why.contains("Play did not write it"), "{why}");
        assert_eq!(fs::read(root.join("gamedata").join("precious.txt")).unwrap(), b"mine");
        remove_tree(&root);
        assert!(game.join("database").join("a.db0").is_file());
    }

    const FSGAME: &str = "$app_data_root$ \t\t= true| false| appdata\\\r\n$arch_dir$              = false| false| $fs_root$|\t\t\t  database\\\r\n$game_data$ \t\t\t= true| true| \t$fs_root$| \t\t\t  gamedata\\\r\n";

    /// An engine (no program: a text file under a harmless name) that says what it can, as the
    /// game's version resource does.
    fn engine_saying(words: &str) -> Vec<u8> {
        let mut bytes = b"an engine; StringFileInfo ".to_vec();
        for s in ["EngineAbilities\0", words, "\0"] {
            bytes.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        }
        bytes
    }

    #[test]
    fn an_engine_that_says_it_can_runs_in_place_and_its_root_holds_no_program() {
        let base = tempfile::tempdir().unwrap();
        let game = game(base.path());
        fs::write(game.join("fsgame.ltx"), FSGAME).unwrap();
        let cx = DeleteContext::default();
        let listed = |dir: &Path| {
            let mut v: Vec<String> = fs::read_dir(dir)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            v.sort();
            v
        };
        let before = listed(&game);

        // a root an older engine ran from: its copies, its link, its content files
        let root = base.path().join("play");
        let old = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap();
        assert_eq!((old.mode, old.programs.len()), (PlayMode::Copied, 2));
        assert_eq!(old.fsltx_args(), ["-fsltx", "fsgame.ltx"]);
        assert!(is_junction(&root.join("database")) && root.join(CONTENT_DIR).is_dir());

        // the game updated to an engine that runs in place
        fs::write(game.join("engine.bin"), engine_saying("fsltx-in-place long-paths")).unwrap();
        let p = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap();
        assert_eq!(p.mode, PlayMode::InPlace);
        assert_eq!(p.exe, game.join("engine.bin"), "the installed engine, where it is");
        assert!(p.programs.is_empty() && p.program_bytes == 0);
        let config = root.join("fsgame.ltx").to_string_lossy().into_owned();
        assert_eq!(p.fsltx_args(), ["-fsltx".to_owned(), config]);
        // no program and no link: the copies and the junction went, the game's archives stay
        assert_eq!(listed(&root), ["appdata", "fsgame.ltx", "gamedata", "modules", MARKER]);
        assert!(game.join("database").join("a.db0").is_file());
        let fsgame = fs::read_to_string(root.join("fsgame.ltx")).unwrap();
        let arch = fsgame.lines().find(|l| l.starts_with("$arch_dir$")).unwrap();
        assert_eq!(
            arch,
            format!("$arch_dir$              = false| false| {}\\|\t\t\t  database\\", game.display())
        );
        assert!(fsgame.contains("$game_data$ \t\t\t= true| true| \t$fs_root$|"), "{fsgame}");
        let marker = fs::read_to_string(root.join(MARKER)).unwrap();
        assert!(!marker.contains("program = ") && !marker.contains("entry = database"), "{marker}");
        assert!(marker.contains("entry = gamedata") && marker.contains("entry = appdata"), "{marker}");
        assert_eq!(fs::read(root.join("appdata").join("user.ltx")).unwrap(), b"vid_mode 800x600");

        // the level is staged as before, and the game's folder is as it was
        let level = base.path().join("built");
        fs::create_dir_all(&level).unwrap();
        fs::write(level.join("level"), b"geometry").unwrap();
        stage_level(&root, "qa_level", &level, b"ai", b"spawn").unwrap();
        assert!(root.join("gamedata").join("levels").join("qa_level").join("level.ai").is_file());
        assert_eq!(listed(&game), before);
        assert!(!game.join("gamedata").join("levels").exists());

        // a new root of an engine in place is made with no program from the start
        let fresh = base.path().join("fresh");
        let p = prepare_with(&input(&game, &fresh, &cx), &PROGRAMS).unwrap();
        assert_eq!((p.mode, p.was), (PlayMode::InPlace, "missing"));
        assert_eq!(listed(&fresh), ["appdata", "fsgame.ltx", "gamedata", "modules", MARKER]);
    }

    #[test]
    fn the_engine_says_how_deep_a_root_may_be() {
        let base = tempfile::tempdir().unwrap();
        let (old, in_place, aware) =
            (base.path().join("a.bin"), base.path().join("b.bin"), base.path().join("c.bin"));
        fs::write(&old, b"an engine that says nothing").unwrap();
        fs::write(&in_place, engine_saying("fsltx-in-place")).unwrap();
        fs::write(&aware, engine_saying("long-paths fsltx-in-place")).unwrap();
        assert_eq!(ability_of(&old), Ability::default());
        assert_eq!(ability_of(&in_place), Ability { in_place: true, long_paths: false });
        assert_eq!(ability_of(&aware), Ability { in_place: true, long_paths: true });
        assert_eq!(ability_of(&base.path().join("missing.bin")), Ability::default());
        let never = || -> bool { panic!("Windows is asked only for a long-path aware engine") };
        assert_eq!(path_limit(ability_of(&old), never), GAME_PATH_LIMIT);
        assert_eq!(path_limit(ability_of(&in_place), never), GAME_PATH_LIMIT);
        assert_eq!(path_limit(ability_of(&aware), || true), 519);
        assert_eq!(path_limit(ability_of(&aware), || false), WINDOWS_PATH_LIMIT);
        // a root an older engine refuses, the long-path aware one takes
        let root = PathBuf::from(format!("C:\\{}", "r".repeat(120)));
        let longest = format!("appdata\\shaders_cache_grizzly\\{}", "s".repeat(150));
        let why = check_path_lengths(&root, &longest, GAME_PATH_LIMIT).unwrap_err();
        assert!(why.contains("too deep") && why.contains("takes 255 at most"), "{why}");
        assert!(check_path_lengths(&root, &longest, 519).is_ok());
    }

    #[test]
    fn the_fs_config_in_place_names_the_games_archives_and_nothing_else_changes() {
        let game = Path::new("D:\\Games\\Dead Air");
        let out = fsgame_in_place(FSGAME, game);
        let lines: Vec<&str> = out.split("\r\n").collect();
        assert_eq!(lines[0], "$app_data_root$ \t\t= true| false| appdata\\");
        assert_eq!(
            lines[1],
            "$arch_dir$              = false| false| D:\\Games\\Dead Air\\|\t\t\t  database\\"
        );
        assert_eq!(lines[2], "$game_data$ \t\t\t= true| true| \t$fs_root$| \t\t\t  gamedata\\");
        assert_eq!(out.len(), FSGAME.len() - "$fs_root$".len() + "D:\\Games\\Dead Air\\".len());
        // an archive folder named outright is left as it is
        let named = "$arch_dir$ = false| false| E:\\archives\\\r\n";
        assert_eq!(fsgame_in_place(named, game), named);
    }
}
