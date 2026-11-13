//! [`ShellBuilder`]: how an area plugs into the frame before the window opens.
//!
//! Each UI crate exposes `pub fn install(b: &mut ShellBuilder)` and `grizzly`'s `main` calls
//! them in turn. The shell names none of them: it draws what was contributed here. Everything is
//! called on the UI thread, so nothing here needs to be `Send`.

use gz_editor::{Editor, SmolStr};
use gz_ui::{Icon, Toolbar};
use rustc_hash::FxHashMap;

use crate::doc::{DocCx, Drawer, Factory};
use crate::keys::Chord;

pub type RunFn = Box<dyn FnMut(&mut DocCx<'_>)>;
pub type EnabledFn = Box<dyn Fn(&Editor, &str) -> bool>;
pub type DrawFn = Box<dyn FnMut(&mut egui::Ui, &mut DocCx<'_>)>;
pub type ToolbarFn = Box<dyn for<'t> FnMut(&mut Toolbar<'_, 't>, &mut DocCx<'_>)>;

/// A command of the UI: what a menu item, a palette row and a shortcut run.
pub struct UiCommand {
    /// The shortcut id, as the Keyboard page and `shortcuts` know it: `COMMAND_UNDO`,
    /// `COMMAND_ZOOM_EXTENTS."Selected"`.
    pub id: SmolStr,
    /// The Keyboard page's group and the palette's category: "Edit", "Mod", ...
    pub group: SmolStr,
    /// The official name: "Undo", "Build Into Game".
    pub label: SmolStr,
    pub icon: Icon,
    /// What it is bound to until the author binds it to something else.
    pub default_keys: Option<Chord>,
    /// It takes a shortcut (the Keyboard page lists it).
    pub bindable: bool,
    /// The palette lists it.
    pub in_palette: bool,
    pub run: RunFn,
    /// Off: the menu greys it out and the palette does not run it. `(editor, active tab)`.
    pub enabled: Option<EnabledFn>,
}

impl UiCommand {
    #[must_use]
    pub fn new(
        id: &str,
        group: &str,
        label: &str,
        icon: Icon,
        run: impl FnMut(&mut DocCx<'_>) + 'static,
    ) -> Self {
        Self {
            id: SmolStr::new(id),
            group: SmolStr::new(group),
            label: SmolStr::new(label),
            icon,
            default_keys: None,
            bindable: true,
            in_palette: true,
            run: Box::new(run),
            enabled: None,
        }
    }

    #[must_use]
    pub fn keys(mut self, keys: &str) -> Self {
        self.default_keys = Chord::parse(keys);
        self
    }

    #[must_use]
    pub fn enabled(mut self, f: impl Fn(&Editor, &str) -> bool + 'static) -> Self {
        self.enabled = Some(Box::new(f));
        self
    }

    /// Takes no shortcut: not on the Keyboard page.
    #[must_use]
    pub fn unbindable(mut self) -> Self {
        self.bindable = false;
        self
    }

    #[must_use]
    pub fn hidden_from_palette(mut self) -> Self {
        self.in_palette = false;
        self
    }
}

/// What a menu row is.
pub enum MenuContent {
    /// A row that runs a command; `label` overrides the command's name ("Open Scene...").
    Command { command: SmolStr, label: Option<SmolStr> },
    /// Rows drawn by their owner: a list that changes (Recent Scene), toggles with checks.
    Custom(DrawFn),
}

/// One contribution to a caption menu.
pub struct MenuEntry {
    /// "File", "Edit", "Mod", "Libraries", "Window", "Help".
    pub menu: SmolStr,
    /// A submenu of it ("Project", "Bake Scene Layer"); `None`: the menu itself.
    pub submenu: Option<SmolStr>,
    /// Rows of one group stand together; a separator stands between groups.
    pub group: i32,
    pub order: i32,
    pub content: MenuContent,
}

/// A drawer's body: what the Content Browser and the Output Log draw over the bottom of a tab.
pub struct DrawerDesc {
    pub label: SmolStr,
    pub icon: Icon,
    /// The Level panel that shows the same object: on Level with it on screen, the drawer
    /// raises it instead.
    pub panel: Option<SmolStr>,
    pub body: DrawFn,
}

/// A dialog's opener: `Err` says why it cannot open.
pub type OpenFn = Box<dyn FnMut(&mut DocCx<'_>) -> Result<(), String>>;
/// What the status bar reads of the log.
pub type LogStateFn = std::sync::Arc<dyn Fn(&Editor) -> LogState + Send + Sync>;
/// Shows a project path in the Content Browser.
pub type RevealFn = Box<dyn FnMut(&str, &mut DocCx<'_>)>;
/// Rebuilds the layout (`true`: every split too).
pub type ResetLayoutFn = Box<dyn FnMut(bool, &mut DocCx<'_>)>;

/// A dialog that opens by id (`ui_dialog open`).
pub struct DialogDesc {
    /// The MCP id: `import_base_scene`, `package_release`, ...
    pub id: SmolStr,
    /// `Title###dlg.<name>`.
    pub window: SmolStr,
    /// It opens only with a project.
    pub needs_project: bool,
    /// Asked to open: `Err` says why it cannot.
    pub open: OpenFn,
    /// Drawn every frame (it knows whether it is open), before the kit's queued questions.
    pub draw: DrawFn,
}

/// A card that takes the Start page over: Link Game, Recovery.
pub struct PageCard {
    pub id: SmolStr,
    /// It wants the page now.
    pub wants: Box<dyn Fn(&Editor) -> bool>,
    pub draw: DrawFn,
}

/// A panel of the Level tab, as the contract names it. Its state is the Level document's.
#[derive(Clone, Debug)]
pub struct PanelDesc {
    pub id: SmolStr,
    pub label: SmolStr,
    pub icon: Icon,
    /// Where it shows in the Compact class.
    pub drawer: Option<Drawer>,
}

/// What the status bar says of the Output Log.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LogState {
    /// The last line worth reading.
    pub last: String,
    pub errors: u32,
    pub warnings: u32,
}

/// Everything the areas plugged in, consumed by the shell when the window opens.
#[derive(Default)]
pub struct ShellBuilder {
    pub(crate) factories: FxHashMap<SmolStr, Factory>,
    pub(crate) commands: Vec<UiCommand>,
    pub(crate) menus: Vec<MenuEntry>,
    pub(crate) project_toolbar: Vec<(i32, ToolbarFn)>,
    pub(crate) drawers: [Option<DrawerDesc>; 2],
    pub(crate) status_chips: Vec<(i32, DrawFn)>,
    pub(crate) start_page: Option<DrawFn>,
    pub(crate) page_cards: Vec<PageCard>,
    pub(crate) dialogs: Vec<DialogDesc>,
    pub(crate) panels: Vec<PanelDesc>,
    pub(crate) log_state: Option<LogStateFn>,
    pub(crate) reveal: Option<RevealFn>,
    pub(crate) reset_layout: Option<ResetLayoutFn>,
    pub(crate) cpp: Vec<crate::cpp::CppCommand>,
}

impl ShellBuilder {
    /// A row of the C++ command table (`list_commands`, `exec_command`): its C++ id, name,
    /// description, sub-commands with their keys, and its action. A row of an id registered twice
    /// keeps the later one; one whose action is not there is left out.
    pub fn cpp_command(&mut self, command: crate::cpp::CppCommand) {
        self.cpp.push(command);
    }

    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What is installed, for a test of an application's own builder: a document kind's
    /// factory, a dialog by its MCP id, a drawer, the Start page.
    #[must_use]
    pub fn has_document(&self, kind: &str) -> bool {
        self.factories.contains_key(kind)
    }

    #[must_use]
    pub fn has_dialog(&self, id: &str) -> bool {
        self.dialogs.iter().any(|d| d.id == id)
    }

    #[must_use]
    pub fn has_drawer(&self, which: Drawer) -> bool {
        self.drawers.get(which.index()).is_some_and(Option::is_some)
    }

    #[must_use]
    pub fn has_start_page(&self) -> bool {
        self.start_page.is_some()
    }

    /// The sub-commands of a C++ table row by its name, as the last registration left it.
    #[must_use]
    pub fn cpp_subs(&self, name: &str) -> Option<Vec<String>> {
        self.cpp
            .iter()
            .rev()
            .find(|c| c.name == name)
            .map(|c| c.subs.iter().map(|s| s.desc.to_string()).collect())
    }

    /// The factory of a document kind. A kind registered twice keeps the later factory.
    pub fn document(
        &mut self,
        kind: &str,
        f: impl FnMut(&std::sync::Arc<Editor>, &crate::OpenRequest) -> Result<Box<dyn crate::Document>, String>
        + 'static,
    ) {
        self.factories.insert(SmolStr::new(kind), Box::new(f));
    }

    /// A command of the UI. An id registered twice keeps the later command.
    pub fn command(&mut self, command: UiCommand) {
        self.commands.retain(|c| c.id != command.id);
        self.commands.push(command);
    }

    /// A menu row running a command.
    pub fn menu_item(&mut self, menu: &str, group: i32, order: i32, command: &str) {
        self.menus.push(MenuEntry {
            menu: SmolStr::new(menu),
            submenu: None,
            group,
            order,
            content: MenuContent::Command { command: SmolStr::new(command), label: None },
        });
    }

    /// Any menu contribution.
    pub fn menu_entry(&mut self, entry: MenuEntry) {
        self.menus.push(entry);
    }

    /// Rows a contribution draws itself (`gz_ui::menu_item` for each).
    pub fn menu_custom(
        &mut self,
        menu: &str,
        group: i32,
        order: i32,
        draw: impl FnMut(&mut egui::Ui, &mut DocCx<'_>) + 'static,
    ) {
        self.menus.push(MenuEntry {
            menu: SmolStr::new(menu),
            submenu: None,
            group,
            order,
            content: MenuContent::Custom(Box::new(draw)),
        });
    }

    /// Items at the right end of every toolbar (Build Mod, Publish Release, Releases), lowest
    /// `order` first. Drawn only with a project and never on Preferences.
    pub fn project_toolbar(
        &mut self,
        order: i32,
        f: impl for<'t> FnMut(&mut Toolbar<'_, 't>, &mut DocCx<'_>) + 'static,
    ) {
        self.project_toolbar.push((order, Box::new(f)));
    }

    pub fn drawer(&mut self, which: Drawer, desc: DrawerDesc) {
        self.drawers[which.index()] = Some(desc);
    }

    /// A chip on the right of the status bar (Rebuild), lowest `order` first, before Tasks and the
    /// counts.
    pub fn status_chip(&mut self, order: i32, draw: impl FnMut(&mut egui::Ui, &mut DocCx<'_>) + 'static) {
        self.status_chips.push((order, Box::new(draw)));
    }

    /// What the work area shows while no project is open (or its game is not linked).
    pub fn start_page(&mut self, draw: impl FnMut(&mut egui::Ui, &mut DocCx<'_>) + 'static) {
        self.start_page = Some(Box::new(draw));
    }

    pub fn page_card(&mut self, card: PageCard) {
        self.page_cards.push(card);
    }

    pub fn dialog(&mut self, desc: DialogDesc) {
        self.dialogs.retain(|d| d.id != desc.id);
        self.dialogs.push(desc);
    }

    pub fn panel(&mut self, desc: PanelDesc) {
        self.panels.retain(|p| p.id != desc.id);
        self.panels.push(desc);
    }

    /// What the status bar and `progress` say of the log. Unset: the editor's log itself.
    pub fn log_state(&mut self, f: impl Fn(&Editor) -> LogState + Send + Sync + 'static) {
        self.log_state = Some(std::sync::Arc::new(f));
    }

    /// A tab's file shown in the Content Browser (the tab menu's Show in Content Browser).
    pub fn reveal_in_browser(&mut self, f: impl FnMut(&str, &mut DocCx<'_>) + 'static) {
        self.reveal = Some(Box::new(f));
    }

    /// Reset Layout: `true` also forgets every split and grid width.
    pub fn reset_layout(&mut self, f: impl FnMut(bool, &mut DocCx<'_>) + 'static) {
        self.reset_layout = Some(Box::new(f));
    }
}
