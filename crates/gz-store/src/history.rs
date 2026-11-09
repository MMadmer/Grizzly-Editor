use std::collections::VecDeque;
use std::sync::Arc;

use gz_base::SmolStr;
use serde::Serialize;

use crate::store::Snap;

/// What the UI and MCP see of a store's history.
#[derive(Clone, Debug, Default, Serialize)]
pub struct HistoryInfo {
    /// Labels of what Undo would take back, newest first.
    pub undo: Vec<String>,
    /// Labels of what Redo would bring back, nearest first.
    pub redo: Vec<String>,
    /// Whether the content differs from what was last saved or loaded.
    pub dirty: bool,
}

pub(crate) struct Step<S> {
    /// The snapshot from BEFORE the edit (on the undo stack) or AFTER it (on the redo stack).
    pub snap: Arc<Snap<S>>,
    pub label: String,
    /// Consecutive edits with one key fold into one step: a drag, a run of typing.
    pub merge: Option<SmolStr>,
}

pub(crate) struct History<S> {
    pub undo: VecDeque<Step<S>>,
    pub redo: Vec<Step<S>>,
    pub limit: usize,
    /// The content serial that is on disk.
    pub saved_edit: u64,
}

impl<S> History<S> {
    pub fn new(limit: usize, saved_edit: u64) -> Self {
        Self { undo: VecDeque::new(), redo: Vec::new(), limit, saved_edit }
    }

    /// Records an edit: `before` is the snapshot the command started from.
    pub fn push(&mut self, before: Arc<Snap<S>>, label: String, merge: Option<SmolStr>) {
        self.redo.clear();
        if merge.is_some() && self.undo.back().is_some_and(|top| top.merge == merge) {
            // same gesture: the step keeps the snapshot from before its first edit
            return;
        }
        self.undo.push_back(Step { snap: before, label, merge });
        while self.undo.len() > self.limit {
            self.undo.pop_front();
        }
    }

    /// Ends a gesture, so that the next edit with the same key starts a new step.
    pub fn seal(&mut self) {
        if let Some(top) = self.undo.back_mut() {
            top.merge = None;
        }
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub fn info(&self, current_edit: u64) -> HistoryInfo {
        HistoryInfo {
            undo: self.undo.iter().rev().map(|s| s.label.clone()).collect(),
            redo: self.redo.iter().rev().map(|s| s.label.clone()).collect(),
            dirty: current_edit != self.saved_edit,
        }
    }
}
