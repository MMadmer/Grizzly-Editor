//! What is asked of a Text tab from outside its frame - a line to go to (Go to Definition into
//! another file, `text.view`), a query to find, a view - kept by the tab's id until the tab takes
//! it on its next frame. A tab that opens because of the ask finds it waiting.

use std::sync::Arc;

use gz_editor::Editor;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::find::Query;

/// How a Markdown file is shown.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Edit,
    Split,
    Preview,
}

impl Mode {
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Edit => "edit",
            Self::Split => "split",
            Self::Preview => "preview",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Edit, Self::Split, Self::Preview].into_iter().find(|m| m.id().eq_ignore_ascii_case(id.trim()))
    }
}

/// A pick list of the tab to put up, or none.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Show {
    Find,
    Symbols,
    Complete,
    None,
}

/// What a tab is asked to do on its next frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ask {
    /// The caret to a 1-based line and column.
    pub goto: Option<(u32, u32)>,
    pub find: Option<Query>,
    pub mode: Option<Mode>,
    pub wrap: Option<bool>,
    pub show: Option<Show>,
}

/// The asks of every Text tab, by tab id.
#[derive(Default)]
pub struct TextTabs {
    asks: Mutex<FxHashMap<String, Ask>>,
}

impl TextTabs {
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(TextTabs::default)
    }

    /// Adds to what the tab of `id` is asked.
    pub fn ask(&self, id: &str, f: impl FnOnce(&mut Ask)) {
        f(self.asks.lock().entry(id.to_owned()).or_default());
    }

    /// What the tab of `id` is asked, taken.
    #[must_use]
    pub fn take(&self, id: &str) -> Option<Ask> {
        self.asks.lock().remove(id)
    }
}
