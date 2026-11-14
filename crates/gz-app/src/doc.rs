//! What a tab is: a [`Document`], and what it is handed while it draws ([`DocCx`]).

use std::any::Any;
use std::sync::Arc;

use egui_wgpu::RenderState;
use gz_editor::{Editor, SmolStr};
use gz_ui::{Icon, Tone, Toolbar};

use crate::keys::Chord;

/// The Edit menu's actions, routed to the active tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditAction {
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
    Find,
}

impl EditAction {
    pub const ALL: [EditAction; 8] = [
        EditAction::Undo,
        EditAction::Redo,
        EditAction::Cut,
        EditAction::Copy,
        EditAction::Paste,
        EditAction::Duplicate,
        EditAction::Delete,
        EditAction::Find,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            EditAction::Undo => "undo",
            EditAction::Redo => "redo",
            EditAction::Cut => "cut",
            EditAction::Copy => "copy",
            EditAction::Paste => "paste",
            EditAction::Duplicate => "duplicate",
            EditAction::Delete => "delete",
            EditAction::Find => "find",
        }
    }
}

/// The two drawers over the bottom of any tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Drawer {
    Content,
    Log,
}

impl Drawer {
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Drawer::Content => "content",
            Drawer::Log => "log",
        }
    }

    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        match id {
            "content" => Some(Drawer::Content),
            "log" => Some(Drawer::Log),
            _ => None,
        }
    }

    #[must_use]
    pub fn window(self) -> &'static str {
        match self {
            Drawer::Content => "Content Drawer###drawer.content",
            Drawer::Log => "Output Log###drawer.log",
        }
    }

    pub(crate) fn index(self) -> usize {
        match self {
            Drawer::Content => 0,
            Drawer::Log => 1,
        }
    }
}

/// What a panel of the Level tab is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelOp {
    Open,
    Close,
    Focus,
    Toggle,
    /// Into a window of its own (only while panels may leave the window).
    Float,
    /// Back into its slot.
    Dock,
}

/// What a panel of the Level tab says of itself, as the last frame left it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelInfo {
    pub open: bool,
    /// Drawn last frame, and the visible tab of its dock node.
    pub visible: bool,
    pub docked: bool,
    /// In a window of its own, outside the editor's.
    pub floating: bool,
    pub focused: bool,
    /// px: x, y, w, h.
    pub rect: Option<[f32; 4]>,
    /// The scroll of the panel, or of its child that scrolls the most (px).
    pub scroll_y: f32,
    pub scroll_max_y: f32,
}

/// What to open: a kind, and for a file kind the file. `id` names a tab that may be there already.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OpenRequest {
    pub kind: String,
    /// Project-relative for a project file; meshes-relative for a model.
    pub path: Option<String>,
    pub id: Option<String>,
    /// What else the kind takes: a texture's source, the Preferences page.
    pub arg: Option<String>,
    /// Bring it up. Off: it opens behind the active tab.
    pub activate: bool,
}

impl OpenRequest {
    #[must_use]
    pub fn kind(kind: &str) -> Self {
        Self { kind: kind.to_owned(), activate: true, ..Self::default() }
    }

    #[must_use]
    pub fn file(kind: &str, path: &str) -> Self {
        Self { kind: kind.to_owned(), path: Some(path.to_owned()), activate: true, ..Self::default() }
    }
}

/// What a document, a command or a contribution asks of the shell; applied after the frame part
/// that asked returns.
#[derive(Default)]
pub struct ShellRequests {
    pub(crate) list: Vec<Request>,
}

pub(crate) enum Request {
    Open(OpenRequest),
    Activate(String),
    /// The UI's close: a tab with unsaved changes asks first.
    Close(String),
    Dialog(String),
    Drawer(Option<Drawer>),
    ToggleDrawer(Drawer),
    Toast(Tone, String),
    Palette,
    Panel(String, PanelOp),
    Exit,
    TaskCard(bool),
    Popup(String),
    OwnKeys(String),
    Action(String),
    CaptureKeys,
}

impl ShellRequests {
    pub fn open(&mut self, req: OpenRequest) {
        self.list.push(Request::Open(req));
    }

    pub fn activate(&mut self, id: &str) {
        self.list.push(Request::Activate(id.to_owned()));
    }

    /// Closes a tab the way its cross does: unsaved changes are asked about first.
    pub fn close(&mut self, id: &str) {
        self.list.push(Request::Close(id.to_owned()));
    }

    /// Opens a dialog registered with [`crate::ShellBuilder::dialog`] by its id.
    pub fn dialog(&mut self, id: &str) {
        self.list.push(Request::Dialog(id.to_owned()));
    }

    pub fn drawer(&mut self, drawer: Option<Drawer>) {
        self.list.push(Request::Drawer(drawer));
    }

    pub fn toggle_drawer(&mut self, drawer: Drawer) {
        self.list.push(Request::ToggleDrawer(drawer));
    }

    pub fn toast(&mut self, tone: Tone, text: &str) {
        self.list.push(Request::Toast(tone, text.to_owned()));
    }

    pub fn palette(&mut self) {
        self.list.push(Request::Palette);
    }

    /// A panel of the Level tab: opening or focusing one brings Level up.
    pub fn panel(&mut self, id: &str, op: PanelOp) {
        self.list.push(Request::Panel(id.to_owned(), op));
    }

    /// Quit, after the question about unsaved work.
    pub fn exit(&mut self) {
        self.list.push(Request::Exit);
    }

    /// Shows or hides the Task Center's card: the button that started a job brings it back.
    pub fn task_card(&mut self, shown: bool) {
        self.list.push(Request::TaskCard(shown));
    }

    /// Puts a popup up by its `ui.popup` name.
    pub fn popup(&mut self, name: &str) {
        self.list.push(Request::Popup(name.to_owned()));
    }

    /// A panel that answers its own keys has the focus this frame (the Content Browser, the
    /// Output Log): the scene's bindings stay away from the next frame's keys.
    pub fn own_keys(&mut self, who: &str) {
        self.list.push(Request::OwnKeys(who.to_owned()));
    }

    /// Runs an action by its id, as its menu row would.
    pub fn action(&mut self, id: &str) {
        self.list.push(Request::Action(id.to_owned()));
    }

    /// The surface reads the keys itself in the next frame (the Keyboard page records a
    /// shortcut): no action, no document and no binding takes one.
    pub fn capture_keys(&mut self) {
        self.list.push(Request::CaptureKeys);
    }
}

/// What a document and a contribution are handed while they draw or run.
pub struct DocCx<'a> {
    pub editor: &'a Arc<Editor>,
    /// The device and queue the window draws with.
    pub gpu: &'a RenderState,
    pub ctx: &'a egui::Context,
    /// The id of the tab that is up.
    pub active_tab: &'a str,
    pub requests: &'a mut ShellRequests,
}

/// A major tab. UI thread only, so not `Send`.
///
/// A document holds VIEW state only: its data lives in its area's store, it draws from
/// snapshots and emits commands (`editor.post`). Only the active document's `body` runs.
#[allow(unused_variables)]
pub trait Document {
    /// "level", "quest:<rel>", "characters", ...
    fn id(&self) -> &str;
    /// One of `ui.contract`'s document kinds.
    fn kind(&self) -> &'static str;
    /// An own asset shows its name without the extension.
    fn label(&self) -> String;
    fn icon(&self) -> Icon;
    fn path(&self) -> Option<String> {
        None
    }
    fn tooltip(&self) -> Option<String> {
        None
    }
    fn closable(&self) -> bool {
        true
    }
    fn dirty(&self) -> bool {
        false
    }
    /// The unsaved work is an area's document, which that area reports, saves and discards
    /// itself (the scene, a graph, the PDA source): the shell does not count it twice.
    fn dirty_elsewhere(&self) -> bool {
        false
    }
    fn missing(&self) -> bool {
        false
    }
    /// The work area keeps 8 pt around the body.
    fn padded(&self) -> bool {
        true
    }
    /// Its owner closed it: the tab goes without a question.
    fn alive(&self) -> bool {
        true
    }
    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {}
    fn body(&mut self, ui: &mut egui::Ui, cx: &mut DocCx<'_>);
    /// The centre of the status bar.
    fn status(&mut self, ui: &mut egui::Ui, cx: &mut DocCx<'_>) {}
    /// Its menus, drawn after Edit.
    fn menus(&mut self, bar: &mut crate::chrome::MenuBar<'_, '_>, cx: &mut DocCx<'_>) {}
    /// Its own keys, after the shell's. `true` when taken.
    fn shortcut(&mut self, cx: &mut DocCx<'_>, chord: &Chord) -> bool {
        false
    }
    /// The Edit menu on this tab. Asked with `run = false` it answers whether the action is
    /// there at all.
    fn edit(&mut self, cx: &mut DocCx<'_>, action: EditAction, run: bool) -> bool {
        false
    }
    fn save(&mut self, cx: &mut DocCx<'_>) -> Result<(), String> {
        Ok(())
    }
    /// Reads its file again (`doc.reload`); `discard` lets it drop unsaved edits first.
    fn reload(&mut self, cx: &mut DocCx<'_>, discard: bool) -> Result<(), String> {
        Err("This tab has no file to read again.".to_owned())
    }
    /// The part of the document that has the keyboard now ("canvas"): actions of that part's
    /// scope take their keys there (`actions::Scope::Part`).
    fn key_part(&self, ctx: &egui::Context) -> Option<&'static str> {
        None
    }
    /// What the toolbar row says for a tab whose toolbar has nothing of its own: "Saved to
    /// mod.ltx as you edit". The row is never drawn empty.
    fn toolbar_status(&self) -> Option<String> {
        None
    }
    /// Forgets the unsaved changes.
    fn discard(&mut self, cx: &mut DocCx<'_>) {}
    fn on_activate(&mut self, cx: &mut DocCx<'_>) {}
    fn on_deactivate(&mut self, cx: &mut DocCx<'_>) {}
    /// The tab is gone: free what it holds.
    fn on_close(&mut self, cx: &mut DocCx<'_>) {}
    /// A tick while another tab is drawn.
    fn inactive(&mut self, cx: &mut DocCx<'_>) {}
    /// What the session gives the factory to reopen it (a project-relative path); `None`: the
    /// id after "<kind>:".
    fn restore_arg(&self) -> Option<String> {
        None
    }
    /// Answers MCP readers that know the kind.
    fn mcp_state(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
    /// The Level tab's panels: what one says of itself. Other tabs have none.
    fn panel_state(&self, id: &str) -> Option<PanelInfo> {
        None
    }
    /// The Level tab's panels: open, close, focus or toggle one. `false` for an id it lacks or
    /// one that cannot do that (the viewport does not close).
    fn panel(&mut self, cx: &mut DocCx<'_>, id: &str, op: PanelOp) -> bool {
        false
    }
    /// The document as `Any`, for its own area to reach it through `UiQueue`.
    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        None
    }
}

/// Makes a document of a kind: the shell calls it when a tab of that kind opens or a restored
/// tab is activated.
pub type Factory = Box<dyn FnMut(&Arc<Editor>, &OpenRequest) -> Result<Box<dyn Document>, String>>;

/// A tab id for a kind and a file: `quest:<rel>` for a file kind, the kind itself for a singleton.
#[must_use]
pub fn tab_id(kind: &str, path: Option<&str>) -> SmolStr {
    match (kind, path) {
        ("quest" | "beh", Some(p)) => SmolStr::new(format!("{kind}:{p}")),
        _ => SmolStr::new(kind),
    }
}

#[cfg(test)]
mod tests {
    use super::{Drawer, EditAction, tab_id};

    #[test]
    fn ids() {
        assert_eq!(tab_id("quest", Some("quests\\a.nqasset")), "quest:quests\\a.nqasset");
        assert_eq!(tab_id("characters", None), "characters");
        assert_eq!(tab_id("pda", Some("x.pdaasset")), "pda");
        assert_eq!(Drawer::parse("log"), Some(Drawer::Log));
        assert_eq!(Drawer::parse("x"), None);
        assert_eq!(EditAction::ALL.len(), 8);
    }
}
