//! What the manifest area keeps while a project is open.
//!
//! [`ManifestService`] (in `session.services`) owns the store of `mod.ltx`. Every editor of the
//! manifest - Mod Settings, World Rules, Screens & Music, the graph tab's globals, Exclude from
//! Build - sends its edits here; [`gz_editor::AutoSave`] puts each published state on disk, and
//! the store's revision is the manifest generation tabs follow. The file may change behind the
//! editor (a git checkout, an agent writing it): the project's watcher says so, the file is read
//! and, when its text is not the store's, loaded as a new document - every tab that follows the
//! revision shows it at once. A command that answers what the file says now calls
//! [`ManifestService::sync`], which reads the disk.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::{Origin, Problem, RelPath, Rev};
use gz_editor::{AutoSave, CmdError, Editor, Executors, ProjectSession, Store};
use gz_project::{Filter, Project, ProjectWatch};
use gz_store::{Apply, Snap, State, StoreOptions};
use parking_lot::{Mutex, RwLock};
use serde_json::Value;

use crate::doc::ManifestDoc;
use crate::edit::{self, Edit};
use crate::exclude;
use crate::io::{installed_commands, read_manifest_file, script_functions};
use crate::model::{FILE, GlobalVar, Manifest, scaffold_text};

struct Loaded {
    doc: ManifestDoc,
    manifest: Manifest,
}

impl Loaded {
    fn new(doc: ManifestDoc) -> Self {
        let manifest = Manifest::parse(&doc);
        Self { doc, manifest }
    }
}

/// The state of the manifest store: the document and its parsed view, or nothing when the
/// project has no `mod.ltx`.
#[derive(Clone, Default)]
pub struct ManifestState {
    loaded: Option<Arc<Loaded>>,
    /// Moves with every edit: a reload read before an edit landed must not undo it.
    serial: u64,
    /// The state is what the disk holds: nothing to write.
    from_disk: bool,
}

impl ManifestState {
    fn of_text(text: Option<String>) -> Self {
        Self {
            loaded: text.map(|t| Arc::new(Loaded::new(ManifestDoc::parse(&t)))),
            serial: 0,
            from_disk: true,
        }
    }

    #[must_use]
    pub fn exists(&self) -> bool {
        self.loaded.is_some()
    }

    /// The document as written: what later areas read their sections from.
    #[must_use]
    pub fn doc(&self) -> Option<&ManifestDoc> {
        self.loaded.as_deref().map(|l| &l.doc)
    }

    #[must_use]
    pub fn manifest(&self) -> Option<&Manifest> {
        self.loaded.as_deref().map(|l| &l.manifest)
    }

    fn text(&self) -> Option<String> {
        self.doc().map(ToString::to_string)
    }
}

/// What the store is told.
pub enum ManifestCmd {
    /// Edits, all or none. Fails when the project has no `mod.ltx`.
    Edit(Vec<Edit>),
    /// The scaffold, when the project has no `mod.ltx` yet; nothing when it has one.
    Create(String),
    /// The file as the disk holds it now, read when the store's edit serial was `serial`.
    Reload { text: Option<String>, serial: u64 },
    /// Wakes the saver so that it notices its owner is gone.
    #[doc(hidden)]
    Release,
}

impl State for ManifestState {
    type Cmd = ManifestCmd;
    /// Whether the document changed.
    type Reply = bool;

    fn apply(&mut self, cmd: ManifestCmd, cx: &mut Apply) -> Result<bool, CmdError> {
        match cmd {
            ManifestCmd::Edit(edits) => {
                let Some(loaded) = &self.loaded else {
                    return Err(CmdError::new("refused", "mod.ltx not found in the project"));
                };
                let before = loaded.doc.to_string();
                let mut doc = loaded.doc.clone();
                for e in edits {
                    edit::apply(&mut doc, e)?;
                }
                if doc.to_string() == before {
                    cx.unchanged();
                    return Ok(false);
                }
                self.loaded = Some(Arc::new(Loaded::new(doc)));
                self.serial += 1;
                self.from_disk = false;
                cx.content_changed();
                Ok(true)
            }
            ManifestCmd::Create(text) => {
                if self.loaded.is_some() {
                    cx.unchanged();
                    return Ok(false);
                }
                self.loaded = Some(Arc::new(Loaded::new(ManifestDoc::parse(&text))));
                self.serial += 1;
                self.from_disk = false;
                cx.new_document();
                Ok(true)
            }
            ManifestCmd::Reload { text, serial } => {
                if serial != self.serial || text == self.text() {
                    cx.unchanged();
                    return Ok(false);
                }
                *self = ManifestState { serial: self.serial + 1, ..ManifestState::of_text(text) };
                cx.new_document();
                Ok(true)
            }
            ManifestCmd::Release => Ok(false),
        }
    }
}

/// The open project's `mod.ltx`. Made on first use by [`ManifestService::of`].
pub struct ManifestService {
    root: PathBuf,
    store: Store<ManifestState>,
    saver: AutoSave,
    exec: Executors,
    /// One sync-and-edit at a time.
    lock: tokio::sync::Mutex<()>,
    /// The project's watcher, once it is followed.
    watched: Mutex<Option<ProjectWatch>>,
}

/// Guards the first use, so that one store is made per project.
struct InitLock(tokio::sync::Mutex<()>);

fn write_error() -> CmdError {
    CmdError::io("can't write mod.ltx")
}

async fn read_text(exec: &Executors, root: &Path) -> Result<Option<String>, CmdError> {
    let root = root.to_path_buf();
    exec.io(move || read_manifest_file(&root))
        .await
        .map_err(|e| CmdError::io(format!("mod.ltx could not be read: {e}")))
}

impl ManifestService {
    /// The open project's manifest service, or `no active project`.
    pub async fn of(editor: &Editor) -> Result<Arc<Self>, CmdError> {
        let session = editor.project()?;
        Self::of_session(editor, &session).await
    }

    /// The manifest service of `session`, made on first use: the file is read once here.
    pub async fn of_session(editor: &Editor, session: &Arc<ProjectSession>) -> Result<Arc<Self>, CmdError> {
        if let Some(found) = session.services.get::<Self>() {
            return Ok(found);
        }
        let init = session.services.get_or_init(|| InitLock(tokio::sync::Mutex::new(())));
        let _one = init.0.lock().await;
        if let Some(found) = session.services.get::<Self>() {
            return Ok(found);
        }
        let text = read_text(&editor.exec, &session.root).await?;
        let service = session.services.insert(Self::new(editor, &session.root, text));
        service.watch(session);
        Ok(service)
    }

    /// The service when it has been made, without waiting: what a frame asks.
    #[must_use]
    pub fn get(session: &ProjectSession) -> Option<Arc<Self>> {
        session.services.get::<Self>()
    }

    fn new(editor: &Editor, root: &Path, text: Option<String>) -> Self {
        let store =
            Store::spawn("manifest", ManifestState::of_text(text), &editor.exec, StoreOptions::default());
        let path = root.join(FILE);
        let log = editor.log.clone();
        let saver = AutoSave::spawn(
            &editor.exec,
            &store,
            move |s: &ManifestState| match s.doc() {
                Some(doc) if !s.from_disk => Some((path.clone(), doc.to_string().into_bytes())),
                _ => None,
            },
            move |e| log.warn(e.to_owned()),
        );
        Self {
            root: root.to_path_buf(),
            store,
            saver,
            exec: editor.exec.clone(),
            lock: tokio::sync::Mutex::new(()),
            watched: Mutex::new(None),
        }
    }

    /// Follows the project's watcher: when `mod.ltx` changes behind the editor (git, an agent,
    /// another program), the disk is read and the store takes it in. The editor's own writes come
    /// back as the text the store already holds and change nothing.
    fn watch(self: &Arc<Self>, session: &ProjectSession) {
        let Some(project) = session.services.get::<Project>() else { return };
        let Ok(rel) = RelPath::new(FILE) else { return };
        let mut rx = project.tree.changes().subscribe(Filter::Path(rel));
        *self.watched.lock() = Some(project.tree.changes().clone());
        let me = Arc::downgrade(self);
        drop(project);
        self.exec.spawn(async move {
            while rx.recv().await.is_some() {
                let Some(me) = me.upgrade() else { break };
                // a file that cannot be read now is read again on its next change
                let _ = me.sync().await;
            }
        });
    }

    /// Whether the project's watcher keeps the store current; without one an edit reads the
    /// disk first.
    fn watched(&self) -> bool {
        self.watched.lock().as_ref().is_some_and(ProjectWatch::live)
    }

    #[must_use]
    pub fn store(&self) -> &Store<ManifestState> {
        &self.store
    }

    #[must_use]
    pub fn snapshot(&self) -> Arc<Snap<ManifestState>> {
        self.store.snapshot()
    }

    /// The manifest generation: moves with every change of the document, an outside one included.
    #[must_use]
    pub fn rev(&self) -> Rev {
        self.store.rev()
    }

    /// The project folder.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Makes the store hold what the disk holds when the file changed behind the editor. The
    /// store's own writes are flushed first, so a pending edit is never taken for an outside one.
    pub async fn sync(&self) -> Result<(), CmdError> {
        let _one = self.lock.lock().await;
        self.sync_locked().await
    }

    async fn sync_locked(&self) -> Result<(), CmdError> {
        if self.saver.flush(self.store.rev()).await.is_err() {
            // what is in memory is not on disk: the disk is not the truth now
            return Ok(());
        }
        let snap = self.store.snapshot();
        let disk = read_text(&self.exec, &self.root).await?;
        if disk != snap.text() {
            self.store.send(ManifestCmd::Reload { text: disk, serial: snap.serial }, Origin::Mcp).await?;
        }
        Ok(())
    }

    /// Applies edits to the manifest and answers once they are on disk. The store is what the
    /// disk holds: the watcher brings outside changes in, and without a watcher the disk is read
    /// first.
    pub async fn edit(&self, edits: Vec<Edit>, origin: Origin) -> Result<bool, CmdError> {
        let _one = self.lock.lock().await;
        if !self.watched() {
            self.sync_locked().await?;
        }
        let changed = self.store.send(ManifestCmd::Edit(edits), origin).await?;
        self.saver.flush(self.store.rev()).await.map_err(|_| write_error())?;
        Ok(changed)
    }

    /// Gives a project without `mod.ltx` its scaffold (a pre-XMS project), then edits it.
    pub async fn ensure(&self, project_name: &str) -> Result<(), CmdError> {
        let _one = self.lock.lock().await;
        self.sync_locked().await?;
        if self.store.snapshot().exists() {
            return Ok(());
        }
        self.store.send(ManifestCmd::Create(scaffold_text(project_name)), Origin::Mcp).await?;
        self.saver.flush(self.store.rev()).await.map_err(|_| write_error())
    }

    /// Whether a project-relative entry is out of the build - the one question every walk that
    /// feeds a build asks (the graphs, the characters, the export).
    #[must_use]
    pub fn outside_build(&self, rel: &str) -> bool {
        let snap = self.store.snapshot();
        let rules = snap.manifest().map_or(&[][..], |m| m.exclude.as_slice());
        exclude::outside_build(rules, rel)
    }

    /// `[vars]` as declared, defaults as written, with the generation they belong to. The
    /// parsed model is kept with the document, so this is a lookup, never a parse.
    #[must_use]
    pub fn globals(&self) -> (Rev, Arc<[GlobalVar]>) {
        let snap = self.store.snapshot();
        let vars = snap.manifest().map_or_else(|| Arc::from(Vec::new()), |m| m.vars.clone());
        (snap.rev, vars)
    }
}

impl Drop for ManifestService {
    fn drop(&mut self) {
        // the saver task holds a handle of the store; a publish lets it see that it is alone
        self.store.post(ManifestCmd::Release, Origin::Mcp);
    }
}

/// What `mod_commands` and the build ask of the project's console commands: the functions its
/// scripts define and the problems of `[commands]` (the linked game's other modules included).
pub async fn command_check(
    editor: &Editor,
    session: &ProjectSession,
    m: &Manifest,
) -> (Vec<String>, Vec<Problem>) {
    let Some(project) = session.services.get::<Project>() else { return (Vec::new(), Vec::new()) };
    project.tree.refresh().await;
    let tree = project.tree.snapshot();
    let root = session.root.clone();
    let game = project.game_root();
    let m = m.clone();
    editor
        .exec
        .io(move || {
            let functions = script_functions(&root, &tree, &m.exclude);
            if m.commands.is_empty() {
                return (functions, Vec::new());
            }
            let taken =
                if game.is_empty() { Vec::new() } else { installed_commands(Path::new(&game), &m.id) };
            let problems = crate::commands::validate_commands(&m, &functions, &taken);
            (functions, problems)
        })
        .await
}

/// Makes a field of the manifest's answer from the state: the world area fills `world`.
pub type FieldHook = Arc<dyn Fn(&ManifestState) -> Value + Send + Sync>;

/// The fields of the manifest's answer that other areas own (`world`). Lives in `editor.services`.
#[derive(Default)]
pub struct ManifestFields {
    hooks: RwLock<Vec<(&'static str, FieldHook)>>,
}

impl ManifestFields {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    /// `field` is a name of the answer (`world`); a later provider replaces an earlier one.
    pub fn provide(&self, field: &'static str, f: impl Fn(&ManifestState) -> Value + Send + Sync + 'static) {
        let mut hooks = self.hooks.write();
        hooks.retain(|(name, _)| *name != field);
        hooks.push((field, Arc::new(f)));
    }

    #[must_use]
    pub fn get(&self, field: &str) -> Option<FieldHook> {
        self.hooks.read().iter().find(|(name, _)| *name == field).map(|(_, f)| f.clone())
    }
}

/// What a key hook of `mod.set` is given.
pub struct ArgCx {
    pub editor: Arc<Editor>,
    /// The project folder.
    pub root: PathBuf,
    /// The argument as sent.
    pub value: String,
    /// The manifest before this call's edits.
    pub manifest: Manifest,
}

/// Turns an argument into edits of the manifest, or a refusal (the C++ text) that stops the
/// whole call before anything is written. May do file work (the thumbnail is converted).
pub type ArgHook = Arc<dyn Fn(ArgCx) -> BoxFuture<'static, Result<Vec<Edit>, String>> + Send + Sync>;

/// The keys of `mod.set` that other areas own: `tags` and `pda.layout` (the Mod
/// Browser's vocabulary and picture). Lives in `editor.services`.
#[derive(Default)]
pub struct ManifestArgs {
    hooks: RwLock<Vec<(&'static str, ArgHook)>>,
}

impl ManifestArgs {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    pub fn provide(
        &self,
        arg: &'static str,
        f: impl Fn(ArgCx) -> BoxFuture<'static, Result<Vec<Edit>, String>> + Send + Sync + 'static,
    ) {
        let mut hooks = self.hooks.write();
        hooks.retain(|(name, _)| *name != arg);
        hooks.push((arg, Arc::new(f)));
    }

    #[must_use]
    pub fn get(&self, arg: &str) -> Option<ArgHook> {
        self.hooks.read().iter().find(|(name, _)| *name == arg).map(|(_, f)| f.clone())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn apply(state: &mut ManifestState, cmd: ManifestCmd) -> bool {
        // the store's Apply is private to gz-store; the state's logic is checked through it
        let rt =
            gz_task::Runtime::start(gz_task::ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 1 })
                .unwrap();
        let store = Store::spawn("test", state.clone(), &rt.exec(), StoreOptions::default());
        let changed = rt.block_on(store.send(cmd, Origin::Mcp)).unwrap();
        *state = store.snapshot().state.clone();
        changed
    }

    #[test]
    fn edits_reloads_and_their_race() {
        let mut s = ManifestState::of_text(Some("[module]\nid = od\n".into()));
        assert!(s.from_disk);
        assert!(apply(&mut s, ManifestCmd::Edit(vec![Edit::set("module", "version", "1.0.0")])));
        assert!(!s.from_disk);
        assert_eq!(s.text().unwrap(), "[module]\nid = od\nversion = 1.0.0\n");
        assert!(!apply(&mut s, ManifestCmd::Edit(vec![Edit::set("module", "version", "1.0.0")])));
        // a reload read before the last edit is dropped
        let stale = s.serial - 1;
        assert!(!apply(
            &mut s,
            ManifestCmd::Reload { text: Some("[module]\nid = other\n".into()), serial: stale }
        ));
        let now = s.serial;
        assert!(apply(
            &mut s,
            ManifestCmd::Reload { text: Some("[module]\nid = other\n".into()), serial: now }
        ));
        assert_eq!(s.manifest().unwrap().id, "other");
        assert!(s.from_disk);
        assert!(!apply(&mut s, ManifestCmd::Create(scaffold_text("x"))));
        let mut none = ManifestState::of_text(None);
        let refused = {
            let rt = gz_task::Runtime::start(gz_task::ExecOptions {
                async_threads: 1,
                cpu_threads: 1,
                io_threads: 1,
            })
            .unwrap();
            let store = Store::spawn("test", none.clone(), &rt.exec(), StoreOptions::default());
            rt.block_on(store.send(ManifestCmd::Edit(vec![Edit::set("module", "id", "x")]), Origin::Mcp))
                .is_err()
        };
        assert!(refused);
        assert!(apply(&mut none, ManifestCmd::Create(scaffold_text("x"))));
        assert_eq!(none.manifest().unwrap().id, "x");
    }
}
