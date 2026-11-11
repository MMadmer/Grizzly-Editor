//! The AI Map, Detail Objects and Wallmark tools (`UIAIMapTool`, `UIDOTool` and the logic under
//! them: `ESceneAIMapTools*.cpp`, `ESceneDOTools*.cpp`, `ESceneWallmarkTools.cpp`).
//!
//! - The data is the scene's [`crate::BulkData`]: the AI map in tiles ([`crate::tools::AiMapPart`],
//!   exact nodes and the packed ones the files hold), the details, the wallmarks.
//! - Every change is one [`BulkEdit`] sent as [`crate::SceneCmd::Bulk`]: the new parts, computed
//!   from a snapshot and refused when the scene moved meanwhile ([`commit`] computes it again; a
//!   generation, which takes long, is dropped and says so). One undo step each, a brush stroke one
//!   merged step; a change of the selection alone is no undo step and leaves nothing unsaved.
//! - The panels' switches ([`BulkToolState`]: Ignore Constraints, Auto Link, the link mode, the
//!   ignored materials, the detail slots' selection) live in [`crate::ToolSettings::bulk`]; Undo
//!   does not take them back.
//! - The work is pure ([`aimap`], [`motion`], [`collide`], [`details`], [`wallmark`]) and runs on
//!   the CPU pool; the generations are jobs of the Task Center (group `scene`).
//! - [`cmd::register`] adds `aimap_tool`, `details_tool` and `wallmark_tool`.

pub mod aimap;
pub mod cmd;
pub mod collide;
pub mod details;
pub mod motion;
pub mod wallmark;
pub mod xr;

use std::sync::Arc;

use glam::Vec3;
use gz_base::{Origin, SmolStr};
use gz_editor::{CmdError, Editor, Value};
use gz_store::Apply;
use parking_lot::RwLock;

pub use aimap::LinkMode;

use crate::apply::{SceneCmd, SceneReply};
use crate::library::Library;
use crate::model::SceneState;
use crate::service::SceneService;
use crate::tools::{AiMapPart, DetailsPart, WallmarkPart};

/// The switches of the AI map panel and the detail slots' selection.
#[derive(Clone, Debug, PartialEq)]
pub struct BulkToolState {
    /// Ignore Constraints: nodes made and linked whatever the height and slope limits say.
    pub ignore_constraints: bool,
    /// Auto Link: a node added links to its neighbours.
    pub auto_link: bool,
    /// What a side button does.
    pub link_mode: LinkMode,
    /// The game materials the AI map is not generated on.
    pub ignored: Vec<SmolStr>,
    /// `m_Selected`: per detail slot.
    pub slots_selected: Arc<[bool]>,
}

impl Default for BulkToolState {
    /// `UIAIMapTool`'s constructor.
    fn default() -> Self {
        Self {
            ignore_constraints: false,
            auto_link: true,
            link_mode: LinkMode::Add,
            ignored: Vec::new(),
            slots_selected: Arc::from([]),
        }
    }
}

/// What kind of change an edit is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// Content: an undo step.
    #[default]
    Step,
    /// Only the selection (node, slot or mark flags): no undo step, nothing unsaved.
    Selection,
    /// Only the panels' switches.
    Switches,
}

/// A change of the bulk tools, one undo step.
#[derive(Clone, Debug, Default)]
pub struct BulkEdit {
    /// The snapshot it was computed from; `None` when it was computed inside the store.
    pub serial: Option<u64>,
    /// `Some(None)` removes the part.
    pub ai_map: Option<Option<Arc<AiMapPart>>>,
    pub details: Option<Option<Arc<DetailsPart>>>,
    pub wallmarks: Option<Option<Arc<WallmarkPart>>>,
    pub tools: Option<BulkToolState>,
    pub kind: Kind,
    pub label: String,
    /// A gesture's key: edits with one key in a row are one undo step.
    pub merge: Option<SmolStr>,
}

impl BulkEdit {
    #[must_use]
    pub fn new(label: &str, serial: u64) -> Self {
        Self { serial: Some(serial), label: label.to_owned(), ..Self::default() }
    }

    #[must_use]
    pub fn selection(mut self) -> Self {
        self.kind = Kind::Selection;
        self
    }

    #[must_use]
    pub fn with_ai(mut self, part: AiMapPart) -> Self {
        self.ai_map = Some(Some(Arc::new(part)));
        self
    }

    #[must_use]
    pub fn with_details(mut self, part: DetailsPart) -> Self {
        self.details = Some(Some(Arc::new(part)));
        self
    }

    #[must_use]
    pub fn with_wallmarks(mut self, part: WallmarkPart) -> Self {
        self.wallmarks = Some(Some(Arc::new(part)));
        self
    }

    fn is_empty(&self) -> bool {
        self.ai_map.is_none() && self.details.is_none() && self.wallmarks.is_none() && self.tools.is_none()
    }
}

/// Applies an edit (the store's side of [`SceneCmd::Bulk`]).
pub(crate) fn apply(state: &mut SceneState, e: BulkEdit, cx: &mut Apply) -> Result<SceneReply, CmdError> {
    if let Some(serial) = e.serial
        && serial != state.serial
    {
        return Err(CmdError::busy("the scene changed meanwhile; ask again"));
    }
    if e.is_empty() {
        cx.unchanged();
        return Ok(SceneReply::None);
    }
    if let Some(a) = e.ai_map {
        state.bulk.ai_map = a;
    }
    if let Some(d) = e.details {
        state.bulk.details = d;
    }
    if let Some(w) = e.wallmarks {
        state.bulk.wallmarks = w;
    }
    if let Some(t) = e.tools {
        state.tools.bulk = t;
    }
    match e.kind {
        Kind::Step => {
            state.serial += 1;
            match e.merge {
                Some(key) => cx.undoable_merged(e.label, key),
                None => cx.undoable(e.label),
            }
        }
        // a computation from a snapshot before the selection changed is stale
        Kind::Selection => state.serial += 1,
        Kind::Switches => {}
    }
    Ok(SceneReply::None)
}

/// What a computed change answers besides the edit.
pub type Outcome = (BulkEdit, Value);

/// Computes a change from the snapshot there is now and sends it; when the scene moved while it
/// was computed the result is dropped and computed again (four times at most). `f` runs on the
/// CPU pool.
pub async fn commit<F>(editor: &Arc<Editor>, svc: &SceneService, origin: Origin, f: F) -> Result<Value, CmdError>
where
    F: Fn(&SceneState, &Library) -> Result<Outcome, CmdError> + Send + Sync + 'static,
{
    let f = Arc::new(f);
    let mut last = CmdError::busy("the scene kept changing; ask again");
    for _ in 0..4 {
        let snap = svc.store.snapshot();
        if !snap.valid {
            return Err(CmdError::new("no_scene", "no scene"));
        }
        let lib = svc.library.clone();
        let g = f.clone();
        let (edit, answer) = editor.exec.cpu(move || g(&snap, &lib)).await?;
        match svc.store.send(SceneCmd::Bulk(Box::new(edit)), origin).await {
            Ok(_) => return Ok(answer),
            Err(e) if e.code == "busy" => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

/// Sets the panels' switches.
pub async fn set_switches(
    svc: &SceneService,
    origin: Origin,
    f: impl FnOnce(&mut BulkToolState),
) -> Result<BulkToolState, CmdError> {
    let mut s = svc.store.snapshot().tools.bulk.clone();
    f(&mut s);
    let edit = BulkEdit { tools: Some(s.clone()), kind: Kind::Switches, ..BulkEdit::default() };
    svc.store.send(SceneCmd::Bulk(Box::new(edit)), origin).await?;
    Ok(s)
}

/// What the viewport's camera is now: where it stands, where it looks, its up and right axes and
/// its heading - what a wallmark is laid by and the AI map's pad turns with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewInfo {
    pub position: Vec3,
    pub forward: Vec3,
    pub top: Vec3,
    pub right: Vec3,
    pub heading: f32,
}

type ViewSource = Box<dyn Fn() -> Option<ViewInfo> + Send + Sync>;

/// The viewport's camera axes, for the tools that must not depend on the viewport. The viewport
/// fills it; until it does, there is none.
#[derive(Default)]
pub struct ViewAxes {
    source: RwLock<Option<ViewSource>>,
}

impl ViewAxes {
    pub fn of(editor: &Editor) -> Arc<ViewAxes> {
        editor.services.get_or_init(ViewAxes::default)
    }

    /// The viewport says how to read its camera; a later call replaces the earlier one.
    pub fn provide(&self, f: impl Fn() -> Option<ViewInfo> + Send + Sync + 'static) {
        *self.source.write() = Some(Box::new(f));
    }

    #[must_use]
    pub fn filled(&self) -> bool {
        self.source.read().is_some()
    }

    #[must_use]
    pub fn get(&self) -> Option<ViewInfo> {
        self.source.read().as_ref().and_then(|f| f())
    }
}
