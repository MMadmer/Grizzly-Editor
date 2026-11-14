//! What a graph takes from the author's preferences when its file does not say: the flow of a
//! graph without positions and the wire options. An `Editor` service: the window keeps it in
//! step with Preferences > Graph, and an editor without a window has the defaults.

use std::sync::Arc;

use gz_editor::Editor;
use parking_lot::RwLock;

use crate::canvas::{Flow, WireSettings};

/// The preferences a graph falls back on.
#[derive(Default)]
pub struct GraphDefaults {
    inner: RwLock<(Flow, WireSettings)>,
}

impl GraphDefaults {
    pub fn of(editor: &Editor) -> Arc<GraphDefaults> {
        editor.services.get_or_init(GraphDefaults::default)
    }

    /// The flow of a new graph, or of one whose file has neither a flow nor positions.
    #[must_use]
    pub fn flow(&self) -> Flow {
        self.inner.read().0
    }

    /// The wire options in force where a graph sets none.
    #[must_use]
    pub fn wires(&self) -> WireSettings {
        self.inner.read().1
    }

    pub fn set(&self, flow: Flow, wires: WireSettings) {
        *self.inner.write() = (flow, wires);
    }
}
