use std::sync::Arc;

use parking_lot::RwLock;

use crate::editor::Editor;

/// Where the viewport's camera stands and the level of the scene it looks at.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CameraPose {
    pub position: [f32; 3],
    /// The open scene's level; `None` without a scene.
    pub level: Option<String>,
}

type Source = Box<dyn Fn() -> Option<CameraPose> + Send + Sync>;

/// The viewport's camera, for surfaces that must not depend on the viewport (a quest's
/// position takes it). The viewport fills it; until it does, there is no camera.
#[derive(Default)]
pub struct ViewCamera {
    source: RwLock<Option<Source>>,
}

impl ViewCamera {
    pub fn of(editor: &Editor) -> Arc<ViewCamera> {
        editor.services.get_or_init(ViewCamera::default)
    }

    /// The viewport says how to read its camera. A later call replaces the earlier one.
    pub fn provide(&self, f: impl Fn() -> Option<CameraPose> + Send + Sync + 'static) {
        *self.source.write() = Some(Box::new(f));
    }

    #[must_use]
    pub fn filled(&self) -> bool {
        self.source.read().is_some()
    }

    /// The camera now; `None` when nobody filled the slot or there is no view.
    #[must_use]
    pub fn get(&self) -> Option<CameraPose> {
        self.source.read().as_ref().and_then(|f| f())
    }
}
