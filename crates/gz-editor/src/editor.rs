use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use arc_swap::ArcSwapOption;
use gz_base::{Origin, Rev, RevCounter, SmolStr};
use gz_log::Log;
use gz_store::CmdError;
use gz_task::{Cancel, Executors, Jobs};
use parking_lot::RwLock;
use serde::Serialize;
use serde_json::Value;

use crate::args::Args;
use crate::frames::FrameClock;
use crate::notices::Notices;
use crate::registry::Registry;
use crate::services::Services;
use crate::spec::Gate;

/// How the editor was started.
#[derive(Clone, Copy, Debug, Default)]
pub struct Flags {
    /// `-nodlg`: no dialog ever opens; questions take their logged answers.
    pub unattended: bool,
    /// No window: everything but the picture.
    pub headless: bool,
    /// `-qa_github`: the network clients talk to the mock named by the environment.
    pub qa_github: bool,
}

/// The folders the editor itself lives in.
#[derive(Clone, Debug, Default)]
pub struct AppPaths {
    /// Where the executable is.
    pub exe_dir: PathBuf,
    /// The editor's data root: `gamedata`, `rawdata`, the project list.
    pub root: PathBuf,
    /// Per-machine scratch of the editor (`_appdata_`).
    pub appdata: PathBuf,
    pub logs: PathBuf,
    /// What the editor writes about itself: `_projects.ini`, `grizzly_editor.ltx` (and `logs`
    /// beside them). The data root unless `-home` names another folder.
    pub home: PathBuf,
}

#[derive(Clone, Debug, Default)]
pub struct EditorOptions {
    pub flags: Flags,
    pub paths: AppPaths,
}

/// An open project: its folder and the services of the areas that work on it. Dropped when the
/// project closes, and every store of the project with it.
pub struct ProjectSession {
    pub root: PathBuf,
    pub services: Services,
}

impl ProjectSession {
    #[must_use]
    pub fn new(root: PathBuf) -> Self {
        Self { root, services: Services::new() }
    }

    /// The path on disk of a project-relative path.
    #[must_use]
    pub fn fs(&self, rel: &gz_base::RelPath) -> PathBuf {
        rel.to_fs(&self.root)
    }
}

/// A document with unsaved changes, as an area reports it.
#[derive(Clone, Debug, Serialize)]
pub struct DirtyDoc {
    /// The document kind: `scene`, `quest`, `pda`, ...
    pub kind: SmolStr,
    /// What the author calls it.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// What saving or discarding an area's documents comes to.
pub type SettleFuture<'a> = Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>;

/// How an area says what unsaved work it holds - asked before anything that would lose it - and
/// saves or drops it on request (`project.close save=true`, `app.window close save=true`).
pub trait DirtySource: Send + Sync {
    fn dirty(&self, editor: &Editor) -> Vec<DirtyDoc>;

    /// Writes every document [`Self::dirty`] reports. The error names the document that could
    /// not be saved and why.
    fn save<'a>(&'a self, editor: &'a Arc<Editor>) -> SettleFuture<'a> {
        Box::pin(async move {
            match self.dirty(editor).first() {
                Some(d) => Err(format!("{}: it cannot be saved on request", d.name)),
                None => Ok(()),
            }
        })
    }

    /// Forgets the unsaved changes of every document [`Self::dirty`] reports. What a source does
    /// not override is dropped with the project.
    fn discard<'a>(&'a self, _editor: &'a Arc<Editor>) -> SettleFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

/// The context of the running editor. One per process, shared as `Arc<Editor>`.
pub struct Editor {
    pub exec: Executors,
    pub log: Log,
    pub jobs: Jobs,
    pub frames: FrameClock,
    pub notices: Notices,
    pub flags: Flags,
    pub paths: AppPaths,
    /// Services that live as long as the editor.
    pub services: Services,
    project: ArcSwapOption<ProjectSession>,
    project_rev: RevCounter,
    registry: Registry,
    dirty: RwLock<Vec<Arc<dyn DirtySource>>>,
    quit: Cancel,
}

impl Editor {
    #[must_use]
    pub fn new(opts: EditorOptions, exec: Executors, log: Log, registry: Registry) -> Arc<Self> {
        let wake = exec.wake().clone();
        let jobs = Jobs::new(exec.clone());
        Arc::new_cyclic(|this: &std::sync::Weak<Self>| {
            // a job started while a project is open is that project's, and stops when it closes
            let weak = this.clone();
            jobs.set_owner_probe(move || {
                let ed = weak.upgrade()?;
                ed.project_opt().map(|_| ed.project_rev().0)
            });
            Self {
                jobs,
                frames: FrameClock::new(wake.clone()),
                notices: Notices::new(log.clone(), wake),
                exec,
                log,
                flags: opts.flags,
                paths: opts.paths,
                services: Services::new(),
                project: ArcSwapOption::empty(),
                project_rev: RevCounter::new(),
                registry,
                dirty: RwLock::new(Vec::new()),
                quit: Cancel::new(),
            }
        })
    }

    /// The open project, or the `no active project` error a command answers with.
    pub fn project(&self) -> Result<Arc<ProjectSession>, CmdError> {
        self.project.load_full().ok_or_else(CmdError::no_project)
    }

    #[must_use]
    pub fn project_opt(&self) -> Option<Arc<ProjectSession>> {
        self.project.load_full()
    }

    #[must_use]
    pub fn project_root(&self) -> Option<PathBuf> {
        self.project.load().as_ref().map(|p| p.root.clone())
    }

    /// Replaces the open project (`None` closes it) and returns the one that was open.
    pub fn set_project(&self, session: Option<Arc<ProjectSession>>) -> Option<Arc<ProjectSession>> {
        let old = self.project.swap(session);
        self.project_rev.bump();
        // what the status line last said was about the project that is gone
        if old.is_some() {
            self.log.forget_last_report();
        }
        // the jobs of the project that closed go with it, with a window or without one
        let stopped = self.jobs.stop_orphans();
        if stopped > 0 {
            self.log.plain(format!("{stopped} job(s) of the closed project were stopped"));
        }
        self.exec.wake().ping();
        old
    }

    /// Moves whenever a project opens or closes.
    #[must_use]
    pub fn project_rev(&self) -> Rev {
        self.project_rev.get()
    }

    /// Whether `path` is the open project's folder.
    #[must_use]
    pub fn is_open_project(&self, path: &Path) -> bool {
        self.project.load().as_ref().is_some_and(|p| p.root == path)
    }

    /// The origin of something the window itself asks for.
    #[must_use]
    pub fn ui_origin(&self) -> Origin {
        if self.flags.unattended { Origin::Unattended } else { Origin::Ui }
    }

    #[must_use]
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Runs a command by its wire name and returns the whole reply object.
    pub async fn call(self: &Arc<Self>, name: &str, args: Args, origin: Origin) -> Value {
        self.registry.call(self.clone(), name, args, origin, Gate::Any).await
    }

    /// [`Self::call`] for a request that came through a door: a command whose effect the door
    /// does not let through answers which door does.
    pub async fn call_gated(self: &Arc<Self>, name: &str, args: Args, origin: Origin, gate: Gate) -> Value {
        self.registry.call(self.clone(), name, args, origin, gate).await
    }

    /// Runs a command from the window without waiting; a failure becomes a notice.
    pub fn post(self: &Arc<Self>, name: &'static str, args: Args) {
        let this = self.clone();
        let origin = self.ui_origin();
        self.exec.spawn(async move {
            let reply = this.call(name, args, origin).await;
            if reply.get("ok").and_then(Value::as_bool) != Some(true) {
                let text =
                    reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.").to_owned();
                this.notices.error(text);
            }
        });
    }

    pub fn add_dirty_source(&self, source: Arc<dyn DirtySource>) {
        self.dirty.write().push(source);
    }

    /// Every document with unsaved changes, across the areas.
    #[must_use]
    pub fn dirty_docs(&self) -> Vec<DirtyDoc> {
        let sources: Vec<Arc<dyn DirtySource>> = self.dirty.read().clone();
        sources.iter().flat_map(|s| s.dirty(self)).collect()
    }

    /// Saves every document with unsaved changes, area by area, and answers what was dirty. The
    /// first save that fails stops it: `Err` names the document and why.
    pub async fn save_all(self: &Arc<Self>) -> Result<Vec<DirtyDoc>, CmdError> {
        let before = self.dirty_docs();
        let sources: Vec<Arc<dyn DirtySource>> = self.dirty.read().clone();
        for source in &sources {
            if !source.dirty(self).is_empty() {
                source.save(self).await.map_err(|e| CmdError::new("save_failed", e))?;
            }
        }
        Ok(before)
    }

    /// Forgets every unsaved change and answers what was dirty.
    pub async fn discard_all(self: &Arc<Self>) -> Result<Vec<DirtyDoc>, CmdError> {
        let before = self.dirty_docs();
        let sources: Vec<Arc<dyn DirtySource>> = self.dirty.read().clone();
        for source in &sources {
            if !source.dirty(self).is_empty() {
                source.discard(self).await.map_err(|e| CmdError::new("discard_failed", e))?;
            }
        }
        Ok(before)
    }

    /// `Err(unsaved)` naming the dirty documents unless there are none or `discard` allows the
    /// loss. What stands in front of everything that closes work.
    pub fn refuse_unsaved(&self, discard: bool) -> Result<(), CmdError> {
        if discard {
            return Ok(());
        }
        let dirty = self.dirty_docs();
        if dirty.is_empty() {
            return Ok(());
        }
        Err(CmdError::unsaved(serde_json::to_value(dirty).unwrap_or(Value::Null)))
    }

    /// Asks the process to end. The window (or the headless loop) watches [`Self::quit_token`].
    pub fn request_quit(&self) {
        self.quit.cancel();
        self.exec.wake().ping();
    }

    #[must_use]
    pub fn quit_token(&self) -> &Cancel {
        &self.quit
    }
}
