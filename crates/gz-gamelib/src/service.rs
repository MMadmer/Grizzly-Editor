//! The library as a service of the editor: which game it is of, its state, the job that brings
//! it up to date (`docs/GAME_LIBRARY.md`, 4).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gz_editor::{CmdError, Editor};
use gz_project::{Machine, Project};
use gz_task::{JobCtx, JobError, JobId, JobState};
use gz_vfs::{GameFiles, MountOptions};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::layout::Layout;
use crate::sync::{self, Options, Progress, Report};

/// The job group of a sync: one runs at a time.
pub const GROUP: &str = "game_library";
/// The job's kind word.
pub const KIND: &str = "game_library";

/// What the last sync of a game came to.
#[derive(Clone, Debug, Default)]
struct Last {
    root: String,
    report: Option<Report>,
    error: Option<String>,
    job: Option<JobId>,
}

/// The game library of the editor.
pub struct GameLibrary {
    appdata: PathBuf,
    /// A game mounted for the library alone: while no project reads it.
    own: Mutex<Option<(String, Arc<GameFiles>)>>,
    last: Mutex<Last>,
    /// The game the watcher last looked at, and the revision of its index then.
    watched: Mutex<(String, u64)>,
    /// When the library was last found current, of which game and which revision of its index:
    /// for a moment after that nobody looks again.
    checked: Mutex<Option<(String, u64, Instant)>>,
}

/// How long a library found current is taken for current without another look: commands that
/// read it come in bursts, and a look reads the whole state and every loose file's date.
const FRESH: Duration = Duration::from_secs(30);

struct JobProgress(JobCtx);

impl Progress for JobProgress {
    fn stage(&self, text: &str) {
        self.0.stage(text);
    }

    fn progress(&self, done: usize, total: usize) {
        self.0.progress(done, total);
    }

    fn stopped(&self) -> bool {
        self.0.cancel.is_cancelled()
    }
}

/// The lock a sync holds on a library: another process that syncs the same game gives way.
struct SyncLock(#[allow(dead_code)] std::fs::File);

impl SyncLock {
    /// Takes the lock, or says that another editor holds it.
    fn take(layout: &Layout) -> Result<Self, String> {
        std::fs::create_dir_all(&layout.root)
            .map_err(|e| format!("the library's folder could not be made: {e}"))?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(layout.lock())
            .map_err(|e| format!("the library's lock could not be opened: {e}"))?;
        match file.try_lock() {
            Ok(()) => Ok(Self(file)),
            Err(_) => Err("another editor is reading this game into the library".to_owned()),
        }
    }
}

impl GameLibrary {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(|| Self {
            appdata: editor.paths.appdata.clone(),
            own: Mutex::new(None),
            last: Mutex::new(Last::default()),
            watched: Mutex::new((String::new(), 0)),
            checked: Mutex::new(None),
        })
    }

    /// The game the library is of now: the open project's, else the editor's default. Empty
    /// when no game is known.
    #[must_use]
    pub fn game_root(editor: &Editor) -> String {
        match Project::of(editor) {
            Ok((_, project)) => project.game_root(),
            Err(_) => Machine::of(editor).default_game(),
        }
    }

    /// The revision of the game's index the open project reads: it moves when the index is read
    /// again (a build that installed a module, a link, a refresh). 0 without a project.
    fn game_rev(editor: &Editor) -> u64 {
        Project::of(editor).map_or(0, |(_, project)| project.game_rev().0)
    }

    /// [`Self::game_root`], the editor's default game looked for first when none is known yet
    /// (a start without a project on a home that never linked one).
    pub async fn find_root(editor: &Editor) -> String {
        let root = Self::game_root(editor);
        if !root.is_empty() || Project::of(editor).is_ok() {
            return root;
        }
        Machine::of(editor).ensure_default_game().await.unwrap_or_default()
    }

    /// The library's folder of a game.
    #[must_use]
    pub fn layout_of(&self, game_root: &str) -> Layout {
        Layout::of(&self.appdata, Path::new(game_root))
    }

    /// The library's folder of the game there is now; `None` when no game is known.
    #[must_use]
    pub fn layout(editor: &Editor) -> Option<Layout> {
        let root = Self::game_root(editor);
        (!root.is_empty()).then(|| Self::of(editor).layout_of(&root))
    }

    /// The index of the game's files: the open project's when it reads this game, else one
    /// mounted for the library (over the saved index). Blocking.
    pub fn files_blocking(&self, editor: &Editor, root: &str) -> Result<Arc<GameFiles>, String> {
        if let Ok((_, project)) = Project::of(editor)
            && project.game_root().eq_ignore_ascii_case(root)
        {
            return project.game_files();
        }
        if let Some((held, files)) = self.own.lock().as_ref()
            && held.eq_ignore_ascii_case(root)
        {
            return Ok(files.clone());
        }
        let opts = MountOptions { cache_dir: Some(self.appdata.join("game_index")) };
        let files = Arc::new(GameFiles::mount(Path::new(root), &opts).map_err(|e| e.to_string())?);
        *self.own.lock() = Some((root.to_owned(), files.clone()));
        Ok(files)
    }

    /// The game's files, off the caller's thread.
    pub async fn files(editor: &Arc<Editor>) -> Result<(String, Arc<GameFiles>), CmdError> {
        let root = Self::find_root(editor).await;
        if root.is_empty() {
            return Err(CmdError::new("no_game", "no game is linked: link one with game.link"));
        }
        let (me, ed, r) = (Self::of(editor), editor.clone(), root.clone());
        let files = editor
            .exec
            .io(move || me.files_blocking(&ed, &r))
            .await
            .map_err(|e| CmdError::new("no_game", e))?;
        Ok((root, files))
    }

    /// The running sync, if one runs.
    #[must_use]
    pub fn running(editor: &Editor) -> Option<gz_task::JobInfo> {
        editor.jobs.running_in(GROUP)
    }

    /// The state for `game.library`: answered without waiting for a file or a job.
    #[must_use]
    pub fn status(editor: &Editor) -> Value {
        let root = Self::game_root(editor);
        let me = Self::of(editor);
        if root.is_empty() {
            return json!({ "state": "no_game", "note": "no game is linked: link one with game.link" });
        }
        let layout = me.layout_of(&root);
        let last = me.last.lock().clone();
        let running = Self::running(editor);
        let made = layout.summary().is_file();
        let state = if running.is_some() {
            "syncing"
        } else if !made {
            "missing"
        } else {
            "ready"
        };
        let mut out = json!({
            "state": state,
            "game": root,
            "folder": layout.root.to_string_lossy(),
            "files": {
                "summary": layout.summary().to_string_lossy(),
                "models": layout.root.join("models").to_string_lossy(),
                "textures": layout.root.join("textures").to_string_lossy(),
                "sounds": layout.root.join("sounds").to_string_lossy(),
                "levels": layout.root.join("levels").to_string_lossy(),
            },
        });
        let Some(m) = out.as_object_mut() else { return out };
        if let Some(job) = running {
            m.insert("job".into(), json!(job.id.0));
            m.insert("stage".into(), json!(job.stage));
            if let Some(f) = job.fraction {
                m.insert("fraction".into(), json!(f));
            }
        }
        if last.root.eq_ignore_ascii_case(&root) {
            if let Some(r) = &last.report {
                m.insert("last_sync".into(), r.to_json());
            }
            if let Some(e) = &last.error {
                m.insert("error".into(), json!(e));
            }
        }
        out
    }

    /// Starts a sync of the game there is now as a job of the editor and answers its id; a sync
    /// that already runs is answered instead. `Err` when no game is known.
    pub fn start(editor: &Arc<Editor>, opts: Options) -> Result<JobId, CmdError> {
        if let Some(job) = Self::running(editor) {
            return Ok(job.id);
        }
        let root = Self::game_root(editor);
        if root.is_empty() {
            return Err(CmdError::new("no_game", "no game is linked: link one with game.link"));
        }
        let me = Self::of(editor);
        let (ed, r, svc) = (editor.clone(), root.clone(), me.clone());
        let started =
            editor.jobs.start_for_editor(KIND, Some(GROUP), "Reading the Game", move |ctx| async move {
                ctx.stage("Reading the game's file index");
                let (svc2, ed2, r2) = (svc.clone(), ed.clone(), r.clone());
                let files =
                    ctx.exec.io(move || svc2.files_blocking(&ed2, &r2)).await.map_err(JobError::new)?;
                let layout = svc.layout_of(&r);
                let lock = match SyncLock::take(&layout) {
                    Ok(l) => l,
                    Err(e) => {
                        svc.note(&r, None, Some(e.clone()));
                        return Err(JobError::new(e));
                    }
                };
                let progress = JobProgress(ctx.clone());
                let (f, l, o) = (files.clone(), layout.clone(), opts.clone());
                let done = ctx.exec.cpu(move || sync::sync(&f, &l, &o, &progress)).await;
                let done = match done {
                    Ok(report) if !report.stopped && o_all(&opts) => {
                        // the levels follow the files: they read the cards the first pass made
                        let progress = JobProgress(ctx.clone());
                        let (f, l, e3) = (files.clone(), layout.clone(), ed.clone());
                        let levels = crate::levels::sync_levels(&e3, &f, &l, opts.rebuild, &progress).await;
                        levels.map(|made| {
                            let mut report = report;
                            report.lists.extend(made.into_iter().map(|l| format!("level {l}")));
                            report
                        })
                    }
                    other => other,
                };
                drop(lock);
                match done {
                    Ok(report) => {
                        let answer = report.to_json();
                        let stopped = report.stopped;
                        svc.note(&r, Some(report), None);
                        crate::index::Index::forget(&ed);
                        if stopped { Err(JobError::from(gz_task::Cancelled)) } else { Ok(answer) }
                    }
                    Err(e) => {
                        svc.note(&r, None, Some(e.clone()));
                        Err(JobError::new(e))
                    }
                }
            });
        match started {
            Ok(id) => {
                me.last.lock().job = Some(id);
                Ok(id)
            }
            Err(busy) => Ok(busy.id),
        }
    }

    fn note(&self, root: &str, report: Option<Report>, error: Option<String>) {
        let mut last = self.last.lock();
        last.root = root.to_owned();
        last.report = report;
        last.error = error;
    }

    /// Starts a sync and waits for it: its report, or why it did not finish.
    pub async fn sync(editor: &Arc<Editor>, opts: Options) -> Result<Value, CmdError> {
        // a game the editor has to find first is found before the job is asked for
        Self::find_root(editor).await;
        let id = Self::start(editor, opts)?;
        match editor.jobs.wait(id).await {
            Some(info) if info.state == JobState::Done => {
                Ok(info.result.map_or_else(|| json!({}), |r| (*r).clone()))
            }
            Some(info) if info.state == JobState::Cancelled => {
                Err(CmdError::new("stopped", "the sync was stopped: the next one goes on where it stopped"))
            }
            Some(info) => Err(CmdError::new(
                "failed",
                info.error.unwrap_or_else(|| "the sync did not finish".to_owned()),
            )),
            None => Err(CmdError::new("failed", "the sync's job is gone")),
        }
    }

    /// The library of the game there is now, made or brought up to date first when it is
    /// missing or the game changed under it. What a command that reads the library calls.
    pub async fn ensure(editor: &Arc<Editor>) -> Result<Layout, CmdError> {
        let (root, files) = Self::files(editor).await?;
        let layout = Self::of(editor).layout_of(&root);
        if let Some(job) = Self::running(editor) {
            let _ = editor.jobs.wait(job.id).await;
        }
        if let Some(why) = Self::behind(editor, &root, &files, &layout).await {
            editor.log.info(format!("the game library is behind the game ({why}): it is brought up to date"));
            Self::sync(editor, Options::default()).await?;
        }
        Ok(layout)
    }

    /// Looks at the game there is now and starts a sync when the library is missing or the
    /// game's files changed. Answers whether one was started.
    pub async fn refresh(editor: &Arc<Editor>) -> bool {
        let Ok((root, files)) = Self::files(editor).await else { return false };
        let layout = Self::of(editor).layout_of(&root);
        let Some(why) = Self::behind(editor, &root, &files, &layout).await else { return false };
        editor.log.info(format!("the game library is behind the game ({why}): it is brought up to date"));
        Self::start(editor, Options::default()).is_ok()
    }

    /// Why the library at `layout` is behind the game, `None` when it is current. A library
    /// found current is not looked at again for [`FRESH`] while the game's index stays the same.
    async fn behind(
        editor: &Arc<Editor>,
        root: &str,
        files: &Arc<GameFiles>,
        layout: &Layout,
    ) -> Option<String> {
        let me = Self::of(editor);
        let rev = Self::game_rev(editor);
        if let Some((r, v, at)) = me.checked.lock().as_ref()
            && r.eq_ignore_ascii_case(root)
            && *v == rev
            && at.elapsed() < FRESH
        {
            return None;
        }
        let (f, l) = (files.clone(), layout.clone());
        let why = editor
            .exec
            .io(move || {
                if !l.summary().is_file() {
                    return Some("it was never made".to_owned());
                }
                sync::stale_reason(&f, &l)
            })
            .await;
        if why.is_none() {
            *me.checked.lock() = Some((root.to_owned(), rev, Instant::now()));
        }
        why
    }
}

/// Whether a sync covers the whole game, so the levels are due after it.
fn o_all(opts: &Options) -> bool {
    opts.only.is_none() && opts.kinds.as_ref().is_none_or(|k| k.iter().any(|x| x == "levels"))
}

/// Keeps the library current: after the editor is up, whenever another game takes the place of
/// the one there was (a link, a project that reads its own game) and whenever the game's index
/// is read again (a build that installed a module, a refresh), the library is checked against
/// the game and synced when it differs.
pub fn watch(editor: &Arc<Editor>) {
    let ed = editor.clone();
    editor.exec.spawn(async move {
        // the first frame and the project of the command line come first
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        // a home that never linked a game looks for one once; a game linked later is the
        // machine's default from then on and is seen below
        GameLibrary::find_root(&ed).await;
        loop {
            if ed.quit_token().is_cancelled() {
                break;
            }
            let root = GameLibrary::game_root(&ed);
            let me = GameLibrary::of(&ed);
            let changed = {
                let watched = me.watched.lock();
                !root.is_empty()
                    && (!watched.0.eq_ignore_ascii_case(&root) || watched.1 != GameLibrary::game_rev(&ed))
            };
            if changed {
                // a model asset over a model of the game is made of a file that may be another now
                gz_media::forget_game_models(&ed);
                GameLibrary::refresh(&ed).await;
                // the look itself may have read the index: what it looked at is what was watched
                *me.watched.lock() = (root, GameLibrary::game_rev(&ed));
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });
}
