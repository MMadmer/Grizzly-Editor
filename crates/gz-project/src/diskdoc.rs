//! A small file the project area keeps in a store and writes as it is edited: the project list,
//! the project's state on this machine, the machine's own file.
//!
//! The file is read again before every edit and the edit is applied to what the disk holds, then
//! written back only when its text changed: QA restores such files behind the editor's back, two
//! editors on one home share them, and a store that only remembered what it wrote would write an
//! old file over a newer one.

use std::path::{Path, PathBuf};

use gz_base::Origin;
use gz_editor::{CmdError, Executors, Store};
use gz_store::{Apply, State, StoreOptions};

/// What such a file is in memory.
pub(crate) trait Doc: Clone + Default + Send + Sync + 'static {
    fn parse(text: &str) -> Self;
    fn to_text(&self) -> String;
}

impl Doc for crate::list::ProjectList {
    fn parse(text: &str) -> Self {
        Self::parse(text)
    }
    fn to_text(&self) -> String {
        Self::to_text(self)
    }
}

impl Doc for crate::machine::ProjectState {
    fn parse(text: &str) -> Self {
        Self::parse(text)
    }
    fn to_text(&self) -> String {
        Self::to_text(self)
    }
}

impl Doc for crate::machine::MachineFile {
    fn parse(text: &str) -> Self {
        Self::parse(text)
    }
    fn to_text(&self) -> String {
        Self::to_text(self)
    }
}

#[derive(Clone)]
pub(crate) struct DocState<D> {
    pub doc: D,
}

impl<D: Doc> State for DocState<D> {
    type Cmd = D;
    type Reply = ();

    fn apply(&mut self, next: D, cx: &mut Apply) -> Result<(), CmdError> {
        if next.to_text() == self.doc.to_text() {
            cx.unchanged();
            return Ok(());
        }
        self.doc = next;
        cx.content_changed();
        Ok(())
    }
}

/// Reads a text file; `""` when it is not there.
pub(crate) async fn read_disk(exec: &Executors, path: &Path) -> Result<String, CmdError> {
    let file = path.to_path_buf();
    match exec.io(move || gz_editor::fsx::read_opt(&file)).await {
        Ok(Some(bytes)) => Ok(gz_base::text::decode_guess(&bytes).into_owned()),
        Ok(None) => Ok(String::new()),
        Err(e) => Err(CmdError::io(format!("{} could not be read: {e}", path.display()))),
    }
}

/// Writes `text` when it differs from what was read: an untouched file keeps every byte. The
/// folder is made when it is missing.
pub(crate) async fn write_disk(
    exec: &Executors,
    path: &Path,
    before: &str,
    text: String,
) -> Result<(), CmdError> {
    if text == before {
        return Ok(());
    }
    let file = path.to_path_buf();
    exec.io(move || {
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        gz_editor::fsx::write_atomic(&file, text.as_bytes())
    })
    .await
    .map_err(|e| CmdError::io(format!("{} could not be saved: {e}", path.display())))
}

/// A file of the home in a store: [`DiskDoc::snapshot`] for readers that must not wait,
/// [`DiskDoc::edit`] to change it.
pub(crate) struct DiskDoc<D: Doc> {
    store: Store<DocState<D>>,
    path: PathBuf,
    exec: Executors,
    /// One read-edit-write at a time.
    lock: tokio::sync::Mutex<()>,
}

impl<D: Doc> DiskDoc<D> {
    pub fn new(name: &'static str, path: PathBuf, initial: D, exec: &Executors) -> Self {
        Self {
            store: Store::spawn(name, DocState { doc: initial }, exec, StoreOptions::default()),
            path,
            exec: exec.clone(),
            lock: tokio::sync::Mutex::new(()),
        }
    }

    /// The file as it was last read or written.
    pub fn snapshot(&self) -> D {
        self.store.snapshot().doc.clone()
    }

    async fn load_locked(&self) -> Result<String, CmdError> {
        let text = read_disk(&self.exec, &self.path).await?;
        self.store.send(D::parse(&text), Origin::Mcp).await?;
        Ok(text)
    }

    /// The file as the disk has it now. A file that cannot be read leaves the last one read.
    pub async fn load(&self) -> Result<D, CmdError> {
        let _one = self.lock.lock().await;
        self.load_locked().await?;
        Ok(self.snapshot())
    }

    /// Applies `f` to the file as the disk has it now and writes the result when it changed.
    pub async fn edit(&self, f: impl FnOnce(&D) -> D) -> Result<D, CmdError> {
        let _one = self.lock.lock().await;
        let before = self.load_locked().await?;
        let next = f(&self.snapshot());
        self.store.send(next, Origin::Mcp).await?;
        let now = self.snapshot();
        write_disk(&self.exec, &self.path, &before, now.to_text()).await?;
        Ok(now)
    }
}
