//! Play's game root (D1): the linked game run on the baked scene without a file of its install
//! changed, by one way only - the installed engine is started where it is, with
//! `-fsltx <root>\fsgame.ltx`. On another fs config Dead Air Refined writes nothing into its own
//! folder (its content records go to the config's app data, a relative root of the config is the
//! config's folder, no update check) and takes the archives from that config's `$arch_dir$`, which
//! the root's `fsgame.ltx` names by the game's own path.
//!
//! The engine says it can, in the version resources of the engine and of the libraries that
//! implement its words (`fsltx-in-place`: [`ability_of`], `gz_game::version::engine_abilities`),
//! read before anything is written or built; nothing is tried. An engine that does not state it,
//! or a mixed installation - a library of an older build beside a newer engine -, is refused in a
//! sentence that names the program ([`ready_engine`]): Play needs a Dead Air Refined build that
//! states it.
//!
//! The root holds the marker, `fsgame.ltx`, the copy of the loose `gamedata` with the level and
//! `editor.spawn` staged into it, an `appdata` of its own (the player's settings copied the first
//! time, no save, no module) and an empty `modules` - no program and no link. The game's own QA
//! roots are built the same way.
//!
//! What may be a root is [`crate::play_root::check_root`]'s: it is asked before anything else
//! here, and a root is made only in a missing or empty folder, which then carries the marker
//! first. Play removes or overwrites an entry of the root's top only when the marker it read at
//! the check listed it or Play wrote it in this very run: each name is claimed - its line
//! appended to the marker - before it is written, and a name found there that is neither is a
//! refusal (a root being made, missing or empty at the check, has nothing removed). What Play
//! writes into its root, and the only things it clears there:
//! - the marker (`play_root.txt`) and its `entry = ` lines;
//! - `fsgame.ltx`, the game's with its `$arch_dir$` naming the game's archives, overwritten;
//! - `gamedata\`, the game's loose gamedata copied anew every time - the old copy removed (a link
//!   in it unlinked) only when the marker listed it and no `project.ltx` / `mod.ltx` lies inside
//!   it, walked again right before -, and the staged level in it;
//! - `appdata\` (logs, saves, screenshots, the player's `user.ltx`, the caches and the game's
//!   content records, seeded once and never cleared), `modules\` (empty).
//!
//! A root an earlier editor ran a copied engine in stays Play's: the programs its marker lists
//! (`program = `), its `database` junction (unlinked, never followed) and its `.dead-air-x64`
//! are removed when Play prepares it.
//!
//! The depth of a root is what the engine takes ([`path_limit`]): a `long-paths` engine 519
//! characters where Windows long paths are on (else `MAX_PATH`, 259), one without the word 255
//! (`CLocatorAPI::Register` copied every path it indexed into a `string256`) - for every path the
//! root holds; the root itself is the game's working folder, which Windows takes up to 258
//! characters ([`WORKING_FOLDER_LIMIT`]).
//!
//! The editor's C++ Play ran IXRay's `Xr3DA.exe`, which read `level.ai.temp` under `-editor_scene`;
//! Dead Air Refined's engine reads `level.ai`, so the staged level folder names it so.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use gz_project::DeleteContext;

use crate::play_root::{self, MARKER, RootState};

/// What of a game folder is its program: the engines a root is looked into for (the linked
/// game's own first) and the libraries that state the engine's words.
#[derive(Clone, Copy, Debug)]
pub struct GamePrograms {
    pub engines: &'static [&'static str],
    /// The libraries beside the engine that implement its words, each stating them in its own
    /// version resource: a word counts only when the engine and every one of them state it.
    pub stating: &'static [&'static str],
}

/// Dead Air Refined's.
pub const LINKED_GAME: GamePrograms = GamePrograms {
    engines: &["xrEngine.exe", "Xr3DA.exe"],
    stating: &["xrEngine.dll", "xrCore.dll", "xrGame.dll"],
};

/// The engine's word for "started on another fs config, I write nothing into my folder and take
/// the archives from that config's `$arch_dir$`".
pub const IN_PLACE: &str = "fsltx-in-place";
/// The engine's word for "long-path aware, my paths take [`gz_game::version::LONG_PATH_LIMIT`]".
pub const LONG_PATHS: &str = "long-paths";

/// What an engine says it can, from the version resources of the engine and of the libraries that
/// implement its words: nothing is started to find out.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ability {
    pub in_place: bool,
    pub long_paths: bool,
    /// The libraries that do not state [`IN_PLACE`] while the engine does, by file name: a mixed
    /// installation.
    pub mixed: Vec<String>,
}

/// The words of a program's version resource; one that cannot be read says nothing.
fn words_of(program: &Path) -> Vec<String> {
    fs::read(program).map(|b| gz_game::version::engine_abilities(&b)).unwrap_or_default()
}

/// [`Ability`] of the engine at `engine`, whose libraries that implement its words
/// (`programs.stating`) lie in `game`: a word counts only when all of them state it, so a library of
/// an older build beside a newer engine makes the installation one that cannot.
#[must_use]
pub fn ability_of(game: &Path, engine: &Path, programs: &GamePrograms) -> Ability {
    let engine_says = words_of(engine);
    let libraries: Vec<(&str, Vec<String>)> =
        programs.stating.iter().map(|l| (*l, words_of(&game.join(l)))).collect();
    let states = |says: &[String], w: &str| says.iter().any(|x| x == w);
    let all = |w: &str| states(&engine_says, w) && libraries.iter().all(|(_, says)| states(says, w));
    let mixed = if states(&engine_says, IN_PLACE) {
        libraries.iter().filter(|(_, says)| !states(says, IN_PLACE)).map(|(l, _)| (*l).to_owned()).collect()
    } else {
        Vec::new()
    };
    Ability { in_place: all(IN_PLACE), long_paths: all(LONG_PATHS), mixed }
}

/// The engine Play starts and what it can.
#[derive(Clone, Debug)]
pub struct Engine {
    pub exe: PathBuf,
    pub ability: Ability,
}

/// The linked game's engine, when it runs in place; otherwise the sentence that tells the author
/// which program of the game does not state [`IN_PLACE`]. Reads the version resources, writes
/// nothing. Blocking.
pub fn ready_engine(game: &Path, programs: &GamePrograms) -> Result<Engine, String> {
    let exe = programs.engines.iter().map(|e| game.join(e)).find(|p| p.is_file()).ok_or_else(|| {
        format!("The linked game at {} has no {}.", game.display(), programs.engines.join(" or "))
    })?;
    let ability = ability_of(game, &exe, programs);
    if ability.in_place {
        return Ok(Engine { exe, ability });
    }
    let name = exe.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Err(match ability.mixed.as_slice() {
        [] => format!(
            "Play runs the linked game where it is installed, and the game's {name} at {} does not state {IN_PLACE}: Play needs a Dead Air Refined build that states it, so update the game.",
            game.display()
        ),
        libraries => format!(
            "Play runs the linked game where it is installed, and {} of the game at {} {} not state {IN_PLACE} though its {name} does (a library of an older build): Play needs a Dead Air Refined build whose programs all state it, so update the game.",
            libraries.join(", "),
            game.display(),
            if libraries.len() == 1 { "does" } else { "do" }
        ),
    })
}

/// The longest path the game takes. `long_paths_on` asks Windows, and only for an engine that
/// is long-path aware.
#[must_use]
pub fn path_limit(ability: &Ability, long_paths_on: impl FnOnce() -> bool) -> usize {
    match (ability.long_paths, ability.long_paths && long_paths_on()) {
        (true, true) => gz_game::version::LONG_PATH_LIMIT,
        (true, false) => WINDOWS_PATH_LIMIT,
        (false, _) => GAME_PATH_LIMIT,
    }
}

/// Whether Windows takes paths over `MAX_PATH` from a long-path aware program.
fn windows_long_paths() -> bool {
    #[cfg(windows)]
    return gz_win::long_paths_enabled();
    #[cfg(not(windows))]
    false
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

/// What a copied engine of an earlier editor kept in its root: its content records and the
/// junction to the game's archives. Removed when Play prepares such a root.
const OLD_CONTENT_DIR: &str = ".dead-air-x64";
const OLD_ARCHIVES_LINK: &str = "database";
/// Caches of the player's `appdata` seeded once, so a Play does not compile every shader again.
const CACHES: [&str; 4] = ["shaders_cache", "shaders_cache_grizzly", "cdb_cache", "render-hardware.cache"];

/// A game root ready to run.
#[derive(Clone, Debug)]
pub struct PlayRoot {
    pub root: PathBuf,
    /// The installed engine, started where it is.
    pub exe: PathBuf,
    /// The longest path the game takes ([`path_limit`]).
    pub path_limit: usize,
    /// What the root was before: `missing`, `empty` or `play_root`.
    pub was: &'static str,
    /// The entries of the old gamedata walked for a project before it was cleared.
    pub walked: u32,
    /// How long that walk took, in milliseconds.
    pub walk_ms: u64,
}

impl PlayRoot {
    /// What goes in front of the caller's parameters: the root's `fsgame.ltx` by its full path,
    /// which the engine takes quoted.
    #[must_use]
    pub fn fsltx_args(&self) -> Vec<String> {
        vec!["-fsltx".to_owned(), self.root.join("fsgame.ltx").to_string_lossy().into_owned()]
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

/// The entries of a root's top Play writes besides the marker.
const ENTRIES: [&str; 4] = ["fsgame.ltx", "gamedata", "appdata", "modules"];
/// Where the engine keeps its content records: the root's app data.
const CONTENT_RECORDS: &str = "appdata\\content\\content-state.txt";

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

    fn claim(&mut self, name: &str) -> Result<(), String> {
        if self.is_ours(name) {
            return Ok(());
        }
        if fs::symlink_metadata(self.root.join(name)).is_ok() {
            return Err(appeared(self.root, name));
        }
        if let Some(m) = self.marker.as_mut() {
            m.write_all(play_root::marker_line(name).as_bytes())
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
    // the engine says it runs in place, or Play does not start it
    let engine = ready_engine(game, programs)?;
    let fsgame = game.join("fsgame.ltx");
    if !fsgame.is_file() {
        return Err(format!("the linked game at {} has no fsgame.ltx", game.display()));
    }
    let text = fs::read(&fsgame).map_err(|e| format!("{} could not be read: {e}", fsgame.display()))?;
    let text = fsgame_in_place(&gz_base::text::cp1251_to_utf8(&text), game);
    let (config, lossy) = gz_base::text::utf8_to_cp1251(&text);
    if lossy {
        return Err(format!(
            "The game's folder {} has a character in its name the game cannot read (it reads paths in the Windows-1251 code page), so its fs config cannot name the game's archives.",
            game.display()
        ));
    }
    let limit = path_limit(&engine.ability, windows_long_paths);
    check_path_lengths(root, &planned_longest(input, state), limit)?;
    let (old_programs, old_entries) = match state {
        RootState::Marked { programs, entries } => (programs.clone(), entries.clone()),
        RootState::Missing | RootState::Empty => (Vec::new(), Vec::new()),
    };
    let listed = |name: &str| old_entries.iter().any(|e| e.eq_ignore_ascii_case(name));

    // from here on the disk changes: the marker first, then each entry claimed before it is written
    let mut claims = Claims::open(root, state)?;
    // the copies an earlier editor ran: Play's own, so they go
    for name in &old_programs {
        let p = root.join(name);
        if fs::symlink_metadata(&p).is_ok_and(|m| m.is_file()) {
            fs::remove_file(&p).map_err(|e| format!("{} could not be removed: {e}", p.display()))?;
        }
    }
    claims.claim("fsgame.ltx")?;
    fs::write(root.join("fsgame.ltx"), &config)
        .map_err(|e| format!("fsgame.ltx could not be written in {}: {e}", root.display()))?;
    // the archives are read where they are: the link a copied engine needed goes, as a link
    let link = root.join(OLD_ARCHIVES_LINK);
    if listed(OLD_ARCHIVES_LINK) && is_junction(&link) {
        remove_tree(&link);
    }
    // the loose gamedata as the game has it now; the old copy goes only from a root whose marker
    // listed it at the check, and only when no project lies inside it
    claims.claim("gamedata")?;
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
    claims.claim("appdata")?;
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
    claims.claim("modules")?;
    let _ = fs::create_dir_all(root.join("modules"));
    // the engine keeps its content records in the root's app data itself
    if listed(OLD_CONTENT_DIR) {
        remove_tree(&root.join(OLD_CONTENT_DIR));
    }
    // the marker as the root is now: the entries written
    drop(claims.marker.take());
    let entries: Vec<String> = ENTRIES.iter().map(|e| (*e).to_owned()).collect();
    fs::write(root.join(MARKER), play_root::marker_text(&entries))
        .map_err(|e| format!("{MARKER} could not be written in {}: {e}", root.display()))?;
    Ok(PlayRoot {
        root: root.to_path_buf(),
        exe: engine.exe,
        path_limit: limit,
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
/// The longest working folder Windows starts a program in (`CreateProcessW` refuses a longer one
/// with `ERROR_DIRECTORY`, a long-path aware caller or not): `MAX_PATH` less the backslash and
/// the terminator. The game runs in its root, so no root is deeper than this.
pub const WORKING_FOLDER_LIMIT: usize = 258;

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
fn planned_longest(input: &PlayInput<'_>, state: &RootState) -> String {
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
        // what the root keeps: gamedata is copied anew and an old link to the archives is not
        // followed
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.eq_ignore_ascii_case("gamedata") || name.eq_ignore_ascii_case(OLD_ARCHIVES_LINK) {
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
    keep_longer(&mut longest, CONTENT_RECORDS.to_owned());
    longest
}

/// Refuses a root the game could not start in: the root itself is the engine's working folder
/// ([`WORKING_FOLDER_LIMIT`]), and `longest`, the longest path it will hold (relative to it), has to
/// fit `limit` ([`path_limit`]) with the root in front.
fn check_path_lengths(root: &Path, longest: &str, limit: usize) -> Result<(), String> {
    let root_text = root.to_string_lossy();
    let depth = root_text.trim_end_matches('\\').chars().count();
    if depth > WORKING_FOLDER_LIMIT {
        return Err(format!(
            "The game root {root_text} is {depth} characters long, and Windows starts a program in a folder of {WORKING_FOLDER_LIMIT} at most: the game runs in its root. Give Play a shorter root."
        ));
    }
    let total = depth + 1 + longest.chars().count();
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

    // no test writes a file named like a program: the game's programs are harmless names here
    const PROGRAMS: GamePrograms = GamePrograms { engines: &["engine.bin"], stating: &["core.lib"] };

    const FSGAME: &str = "$app_data_root$ \t\t= true| false| appdata\\\r\n$arch_dir$              = false| false| $fs_root$|\t\t\t  database\\\r\n$game_data$ \t\t\t= true| true| \t$fs_root$| \t\t\t  gamedata\\\r\n";

    /// A program (no program: a text file under a harmless name) that states `words`, as the
    /// game's version resource does.
    fn saying(words: &str) -> Vec<u8> {
        let mut bytes = b"a program; StringFileInfo ".to_vec();
        for s in ["EngineAbilities\0", words, "\0"] {
            bytes.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        }
        bytes
    }

    /// A game whose engine and library state that they run in place.
    fn game(base: &Path) -> PathBuf {
        let game = base.join("game");
        fs::create_dir_all(game.join("gamedata").join("configs")).unwrap();
        fs::create_dir_all(game.join("database")).unwrap();
        fs::create_dir_all(game.join("appdata").join("savedgames")).unwrap();
        fs::create_dir_all(game.join("modules").join("someone")).unwrap();
        fs::write(game.join("engine.bin"), saying("fsltx-in-place long-paths")).unwrap();
        fs::write(game.join("core.lib"), saying("long-paths fsltx-in-place")).unwrap();
        fs::write(game.join("readme.txt"), b"not copied").unwrap();
        fs::write(game.join("fsgame.ltx"), FSGAME).unwrap();
        fs::write(game.join("gamedata").join("configs").join("system.ltx"), b"[x]").unwrap();
        fs::write(game.join("database").join("a.db0"), b"db").unwrap();
        fs::write(game.join("appdata").join("user.ltx"), b"vid_mode 800x600").unwrap();
        fs::write(game.join("appdata").join("savedgames").join("s.scop"), b"save").unwrap();
        game
    }

    fn input<'a>(game: &'a Path, root: &'a Path, cx: &'a DeleteContext) -> PlayInput<'a> {
        PlayInput { game, root, context: cx, archived: "", level: None }
    }

    fn listed(dir: &Path) -> Vec<String> {
        let mut v: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    }

    fn marked(entries: &[&str], programs: &[&str]) -> String {
        let mut text = play_root::marker_head().to_owned();
        for p in programs {
            text.push_str(&format!("program = {p}\r\n"));
        }
        for e in entries {
            text.push_str(&format!("entry = {e}\r\n"));
        }
        text
    }

    #[test]
    fn a_play_root_runs_the_installed_engine_and_holds_no_program() {
        let base = tempfile::tempdir().unwrap();
        let game = game(base.path());
        let before = listed(&game);
        let cx = DeleteContext::default();
        let root = base.path().join("play");
        let p = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap();
        assert_eq!(p.exe, game.join("engine.bin"), "the installed engine, where it is");
        assert_eq!(p.was, "missing");
        let config = root.join("fsgame.ltx").to_string_lossy().into_owned();
        assert_eq!(p.fsltx_args(), ["-fsltx".to_owned(), config]);
        assert_eq!(listed(&root), ["appdata", "fsgame.ltx", "gamedata", "modules", MARKER]);
        let fsgame = fs::read_to_string(root.join("fsgame.ltx")).unwrap();
        let arch = fsgame.lines().find(|l| l.starts_with("$arch_dir$")).unwrap();
        assert_eq!(
            arch,
            format!("$arch_dir$              = false| false| {}\\|\t\t\t  database\\", game.display())
        );
        let marker = fs::read_to_string(root.join(MARKER)).unwrap();
        assert!(!marker.contains("program = ") && !marker.contains("entry = database"), "{marker}");
        assert!(marker.contains("entry = gamedata") && marker.contains("entry = fsgame.ltx"), "{marker}");
        assert!(root.join("gamedata").join("configs").join("system.ltx").is_file());
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

        // again, after the author left a file in the root: it and the root's settings stay
        fs::write(root.join("notes.txt"), b"mine").unwrap();
        fs::write(root.join("appdata").join("user.ltx"), b"vid_mode 1024x768").unwrap();
        let again = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap();
        assert_eq!(again.was, "play_root");
        assert_eq!(fs::read(root.join("notes.txt")).unwrap(), b"mine");
        assert_eq!(fs::read(root.join("appdata").join("user.ltx")).unwrap(), b"vid_mode 1024x768");
        assert!(!root.join("gamedata").join("levels").exists(), "a new Play stages its level anew");
        // the game's folder is as it was
        assert_eq!(listed(&game), before);
        assert!(!game.join("gamedata").join("levels").exists());
        assert_eq!(fs::read_to_string(game.join("fsgame.ltx")).unwrap(), FSGAME);
    }

    #[test]
    fn an_engine_that_cannot_is_refused_before_anything_is_written() {
        let base = tempfile::tempdir().unwrap();
        let game = game(base.path());
        let cx = DeleteContext::default();
        let root = base.path().join("play");
        // an engine of an older build: says nothing
        fs::write(game.join("engine.bin"), b"an engine that says nothing").unwrap();
        let why = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap_err();
        assert!(
            why.contains("the game's engine.bin at") && why.contains("does not state fsltx-in-place"),
            "{why}"
        );
        assert!(
            why.contains("Dead Air Refined build that states it") && why.contains("update the game"),
            "{why}"
        );
        assert!(!root.exists(), "nothing was made");
        // a mixed installation: a newer engine beside a library of an older build, named
        fs::write(game.join("engine.bin"), saying("fsltx-in-place long-paths")).unwrap();
        fs::write(game.join("core.lib"), b"a library of an older build").unwrap();
        let why = ready_engine(&game, &PROGRAMS).unwrap_err();
        assert!(
            why.contains("core.lib of the game at") && why.contains("does not state fsltx-in-place"),
            "{why}"
        );
        assert!(why.contains("though its engine.bin does"), "{why}");
        assert_eq!(prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap_err(), why);
        assert!(!root.exists(), "nothing was made");
        // a game without its engine
        fs::remove_file(game.join("engine.bin")).unwrap();
        let why = ready_engine(&game, &PROGRAMS).unwrap_err();
        assert!(why.contains("has no engine.bin"), "{why}");
    }

    #[cfg(windows)]
    #[test]
    fn a_root_an_earlier_editor_ran_copies_in_is_taken_over() {
        let base = tempfile::tempdir().unwrap();
        let game = game(base.path());
        let before = listed(&game);
        let cx = DeleteContext::default();
        // the root an earlier Play made for a copied engine: its copies, its link, its content files
        let root = base.path().join("play");
        fs::create_dir_all(root.join("gamedata")).unwrap();
        fs::create_dir_all(root.join(".dead-air-x64")).unwrap();
        fs::create_dir_all(root.join("appdata")).unwrap();
        fs::create_dir_all(root.join("modules")).unwrap();
        fs::write(root.join("engine.bin"), b"a copy of the engine").unwrap();
        fs::write(root.join("core.lib"), b"a copy of the library").unwrap();
        fs::write(root.join("fsgame.ltx"), b"$fs_root$").unwrap();
        fs::write(root.join(".dead-air-x64").join("content-state.txt"), b"state").unwrap();
        fs::write(root.join("notes.txt"), b"mine").unwrap();
        let made = crate::process::console("cmd")
            .args(["/c", "mklink", "/J"])
            .arg(root.join("database"))
            .arg(game.join("database"))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(made.success() && is_junction(&root.join("database")));
        let entries = ["fsgame.ltx", "database", "gamedata", "appdata", "modules", ".dead-air-x64"];
        fs::write(root.join(MARKER), marked(&entries, &["core.lib", "engine.bin"])).unwrap();

        let p = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap();
        assert_eq!((p.was, p.exe.clone()), ("play_root", game.join("engine.bin")));
        // no program and no link: the copies and the junction went, the game's archives stay
        assert_eq!(listed(&root), ["appdata", "fsgame.ltx", "gamedata", "modules", "notes.txt", MARKER]);
        assert!(game.join("database").join("a.db0").is_file());
        let marker = fs::read_to_string(root.join(MARKER)).unwrap();
        assert!(!marker.contains("program = ") && !marker.contains("entry = database"), "{marker}");
        assert!(!marker.contains(".dead-air-x64"), "{marker}");
        assert_eq!(listed(&game), before);
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
        let longest = planned_longest(&fits, &RootState::Missing);
        assert!(check_path_lengths(&root, &longest, GAME_PATH_LIMIT).is_ok());
        let deep = "a".repeat(room + 1);
        let older = GamePrograms { stating: &[], ..PROGRAMS };
        fs::write(game.join("engine.bin"), saying("fsltx-in-place")).unwrap();
        let refused =
            prepare_with(&PlayInput { archived: &deep, ..input(&game, &root, &cx) }, &older).unwrap_err();
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
        // missing at the check, a modules file and fsgame.ltx of someone's there now: neither
        // overwritten
        for (name, text) in
            [("modules", b"someone's file".as_slice()), ("fsgame.ltx", b"someone's".as_slice())]
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
        fs::write(root.join(MARKER), marked(&["gamedata"], &[])).unwrap();
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
        fs::write(root.join(MARKER), marked(&[], &["engine.bin"])).unwrap();
        fs::write(root.join("gamedata").join("precious.txt"), b"mine").unwrap();
        let why = prepare_with(&input(&game, &root, &cx), &PROGRAMS).unwrap_err();
        assert!(why.contains("is in the game root") && why.contains("Play did not write it"), "{why}");
        assert_eq!(fs::read(root.join("gamedata").join("precious.txt")).unwrap(), b"mine");
        remove_tree(&root);
        assert!(game.join("database").join("a.db0").is_file());
    }

    #[test]
    fn the_engine_says_how_deep_a_root_may_be() {
        let base = tempfile::tempdir().unwrap();
        let (old, in_place, aware) =
            (base.path().join("a.bin"), base.path().join("b.bin"), base.path().join("c.bin"));
        fs::write(&old, b"an engine that says nothing").unwrap();
        fs::write(&in_place, saying("fsltx-in-place")).unwrap();
        fs::write(&aware, saying("long-paths fsltx-in-place")).unwrap();
        // the engine alone states the words here: no library implements them
        let alone = GamePrograms { stating: &[], ..PROGRAMS };
        let of = |engine: &Path| ability_of(base.path(), engine, &alone);
        assert_eq!(of(&old), Ability::default());
        assert_eq!(of(&in_place), Ability { in_place: true, long_paths: false, mixed: Vec::new() });
        assert_eq!(of(&aware), Ability { in_place: true, long_paths: true, mixed: Vec::new() });
        assert_eq!(of(&base.path().join("missing.bin")), Ability::default());
        let never = || -> bool { panic!("Windows is asked only for a long-path aware engine") };
        assert_eq!(path_limit(&of(&in_place), never), GAME_PATH_LIMIT);
        assert_eq!(path_limit(&of(&aware), || true), 519);
        assert_eq!(path_limit(&of(&aware), || false), WINDOWS_PATH_LIMIT);
        // a word counts only when every library that implements it states it too
        let two = GamePrograms { stating: &["core.lib", "game.lib"], ..PROGRAMS };
        fs::write(base.path().join("core.lib"), saying("fsltx-in-place long-paths")).unwrap();
        fs::write(base.path().join("game.lib"), saying("fsltx-in-place")).unwrap();
        let got = ability_of(base.path(), &aware, &two);
        assert_eq!(got, Ability { in_place: true, long_paths: false, mixed: Vec::new() });
        assert_eq!(path_limit(&got, never), GAME_PATH_LIMIT, "an older library's paths are the limit");
        fs::write(base.path().join("game.lib"), b"a library of an older build").unwrap();
        let got = ability_of(base.path(), &aware, &two);
        assert_eq!((got.in_place, got.long_paths, got.mixed), (false, false, vec!["game.lib".to_owned()]));
        fs::remove_file(base.path().join("core.lib")).unwrap();
        assert_eq!(
            ability_of(base.path(), &aware, &two).mixed,
            ["core.lib", "game.lib"],
            "a missing one says nothing"
        );
        // an engine that does not state it names no library
        assert!(ability_of(base.path(), &old, &two).mixed.is_empty());
        // a root an engine without long paths refuses, the long-path aware one takes
        let root = PathBuf::from(format!("C:\\{}", "r".repeat(120)));
        let longest = format!("appdata\\shaders_cache_grizzly\\{}", "s".repeat(150));
        let why = check_path_lengths(&root, &longest, GAME_PATH_LIMIT).unwrap_err();
        assert!(why.contains("too deep") && why.contains("takes 255 at most"), "{why}");
        assert!(check_path_lengths(&root, &longest, WINDOWS_PATH_LIMIT).is_err());
        assert!(check_path_lengths(&root, &longest, 519).is_ok());
        // the root is the game's working folder: never deeper than Windows starts a program in
        let deepest = PathBuf::from(format!("C:\\{}", "r".repeat(WORKING_FOLDER_LIMIT - 3)));
        assert!(check_path_lengths(&deepest, "fsgame.ltx", 519).is_ok());
        let why = check_path_lengths(&PathBuf::from(format!("{}r\\", deepest.display())), "fsgame.ltx", 519)
            .unwrap_err();
        assert!(why.contains("259 characters long") && why.contains("a folder of 258 at most"), "{why}");
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
