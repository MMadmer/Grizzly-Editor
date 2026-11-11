//! Unsaved work that lives in a tab alone - the Characters tab, the libraries - as the editor's
//! `DirtySource`, so every command that closes work counts it, saves it and drops it like an
//! area's documents. A tab whose work an area reports itself (`Document::dirty_elsewhere`: the
//! scene, the graphs, the PDA source) is left to that area.
//!
//! The UI thread writes the list of such tabs each frame it changes; the source reads it from any
//! thread. A save asks each tab to save through the UI queue and then waits, frame by frame, until
//! the tabs are clean: a library or a character writes on the pools, after its `save` returned.

use std::sync::Arc;
use std::time::Duration;

use gz_editor::{DirtyDoc, DirtySource, Editor, SettleFuture, SmolStr};
use parking_lot::Mutex;

use crate::queue::UiQueue;
use crate::shell::ShellState;

/// How long a save on request may take before the command gives up on it.
const SAVE_WAIT: Duration = Duration::from_secs(120);

/// A tab with unsaved work only it holds: `(id, label, kind)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DirtyTab {
    pub id: String,
    pub label: String,
    pub kind: &'static str,
}

/// The tabs with unsaved work of their own, as the last frame saw them.
#[derive(Default)]
pub(crate) struct TabDirtyList(Mutex<Vec<DirtyTab>>);

impl TabDirtyList {
    fn get(&self) -> Vec<DirtyTab> {
        self.0.lock().clone()
    }
}

struct TabsDirty;

fn list(editor: &Editor) -> Vec<DirtyTab> {
    editor.services.get::<TabDirtyList>().map(|l| l.get()).unwrap_or_default()
}

impl DirtySource for TabsDirty {
    fn dirty(&self, editor: &Editor) -> Vec<DirtyDoc> {
        list(editor)
            .into_iter()
            .map(|t| DirtyDoc { kind: SmolStr::new(t.kind), name: t.label, path: Some(t.id) })
            .collect()
    }

    fn save<'a>(&'a self, editor: &'a Arc<Editor>) -> SettleFuture<'a> {
        Box::pin(async move {
            let ids: Vec<String> = list(editor).into_iter().map(|t| t.id).collect();
            save_tabs(editor, ids).await
        })
    }

    fn discard<'a>(&'a self, editor: &'a Arc<Editor>) -> SettleFuture<'a> {
        Box::pin(async move {
            let ids: Vec<String> = list(editor).into_iter().map(|t| t.id).collect();
            let Ok(queue) = UiQueue::of(editor) else { return Ok(()) };
            queue
                .run(move |shell, ctx| {
                    for id in &ids {
                        shell.discard_tab(ctx, id);
                    }
                    Ok(())
                })
                .await
                .map_err(|e| e.message)
        })
    }
}

/// Registers the tabs' source once.
pub(crate) fn install(editor: &Editor) {
    editor.services.get_or_init(TabDirtyList::default);
    editor.add_dirty_source(Arc::new(TabsDirty));
}

impl ShellState {
    /// The UI thread's half: the list of tabs with work of their own, written when it changed.
    pub(crate) fn publish_dirty_tabs(&self) {
        let now: Vec<DirtyTab> = self
            .tabs
            .iter()
            .filter_map(|t| {
                let doc = t.doc.as_ref()?;
                (doc.dirty() && !doc.dirty_elsewhere()).then(|| DirtyTab {
                    id: t.id.to_string(),
                    label: t.label(),
                    kind: doc.kind(),
                })
            })
            .collect();
        if let Some(l) = self.editor.services.get::<TabDirtyList>() {
            let mut cur = l.0.lock();
            if *cur != now {
                *cur = now;
            }
        }
    }
}

/// Saves the tabs `ids` and waits until each is clean: `Err` names the first that refused, or
/// the one whose save did not end in time.
pub async fn save_tabs(editor: &Arc<Editor>, ids: Vec<String>) -> Result<(), String> {
    if ids.is_empty() {
        return Ok(());
    }
    let queue = UiQueue::of(editor).map_err(|e| e.message)?;
    let asked = ids.clone();
    queue
        .run(move |shell, ctx| {
            for id in &asked {
                if let Err(e) = shell.save_tab(ctx, id) {
                    let label = shell
                        .find(id)
                        .and_then(|i| shell.tabs.get(i))
                        .map(crate::shell::Tab::label)
                        .unwrap_or_else(|| id.clone());
                    return Err(gz_editor::CmdError::new("save_failed", format!("{label}: {e}")));
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| e.message)?;
    let deadline = tokio::time::Instant::now() + SAVE_WAIT;
    loop {
        let ids = ids.clone();
        let left: Vec<String> = queue
            .run(move |shell, _| {
                Ok(ids
                    .iter()
                    .filter(|id| {
                        shell.find(id).and_then(|i| shell.tabs.get(i)).is_some_and(crate::shell::Tab::dirty)
                    })
                    .map(|id| {
                        shell
                            .find(id)
                            .and_then(|i| shell.tabs.get(i))
                            .map(crate::shell::Tab::label)
                            .unwrap_or_default()
                    })
                    .collect())
            })
            .await
            .map_err(|e| e.message)?;
        let Some(first) = left.first() else { return Ok(()) };
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("{first}: the save did not finish"));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
