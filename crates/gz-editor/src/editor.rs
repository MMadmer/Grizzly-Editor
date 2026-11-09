use std::path::{Path, PathBuf};
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

/// How an area says what unsaved work it holds. Asked before anything that would lose it.
pub trait DirtySource: Send + Sync {
    fn dirty(&self, editor: &Editor) -> Vec<DirtyDoc>;
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
        Arc::new(Self {
            jobs: Jobs::new(exec.clone()),
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
        self.registry.call(self.clone(), name, args, origin).await
    }

    /// Runs a command from the window without waiting; a failure becomes a notice.
    pub fn post(self: &Arc<Self>, name: &'static str, args: Args) {
        let this = self.clone();
        let origin = self.ui_origin();
        self.exec.spawn(async move {
            let reply = this.call(name, args, origin).await;
            if reply.get("ok").and_then(Value::as_bool) != Some(true) {
                let text = reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.").to_owned();
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
