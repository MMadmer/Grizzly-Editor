//! [`ShellBuilder`]: how an area plugs into the frame before the window opens.
//!
//! Each UI crate exposes `pub fn install(b: &mut ShellBuilder)` and `grizzly`'s `main` calls
//! them in turn. The shell names none of them: it draws what was contributed here. Everything is
//! called on the UI thread, so nothing here needs to be `Send`.
//!
//! What the author triggers by name is an [`Action`] (`ShellBuilder::action`): a menu row names
//! it (`menu_action`), a toolbar draws it (`actions::toolbar_button`), the palette and the
//! Keyboard page list it.

use std::sync::Arc;

use gz_editor::{Editor, SmolStr};
use gz_ui::{Icon, Toolbar};
use rustc_hash::FxHashMap;

use crate::actions::Action;
use crate::doc::{DocCx, Drawer, Factory};

pub type DrawFn = Box<dyn FnMut(&mut egui::Ui, &mut DocCx<'_>)>;
pub type ToolbarFn = Box<dyn for<'t> FnMut(&mut Toolbar<'_, 't>, &mut DocCx<'_>)>;

/// What a menu row is.
pub enum MenuContent {
    /// A row that runs an action; `label` overrides its title ("Build to Folder...").
    Action { id: SmolStr, label: Option<SmolStr> },
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
/// Rebuilds the layout (`true`: every split too).
pub type ResetLayoutFn = Box<dyn FnMut(bool, &mut DocCx<'_>)>;

/// A dialog that opens by id (`ui.dialog open`).
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

/// A group of rows an area adds to a page of Preferences: the page (an existing one -
/// General, Interface, Viewport, Tools, Objects, Recovery - or one of its own: Game, Library,
/// Graph), a section title (empty for rows of the page itself), and the rows.
#[derive(Clone, Copy, Debug)]
pub struct PrefGroup {
    pub page: &'static str,
    /// The page's icon in the nav, for a page this group brings.
    pub icon: Icon,
    pub section: &'static str,
    pub rows: &'static [PrefRow],
    /// What the group says instead of its rows while they mean nothing here ("This is a
    /// development build: it does not update itself."); `None` from it shows the rows.
    pub note: Option<NoteFn>,
}

/// What a group says instead of its rows, as one read-only row - its label and its sentence
/// (`None`: the rows). The sentence says whether it is the area's own words or data.
pub type NoteFn = fn(&Arc<Editor>) -> Option<(&'static str, gz_ui::Tx<'static>)>;

/// A row of a [`PrefGroup`]: its official label, its help, and what it is.
#[derive(Clone, Copy, Debug)]
pub struct PrefRow {
    pub label: &'static str,
    pub help: &'static str,
    pub kind: PrefKind,
}

/// What a row of an area's group edits or shows.
#[derive(Clone, Copy, Debug)]
pub enum PrefKind {
    /// A setting of the editor's settings file, edited as its type says; `app.prefs` lists and
    /// sets it. `choices` names the values of a `U32` setting picked from a list (index = value).
    Setting { def: crate::settings::PrefDef, unit: &'static str, choices: &'static [&'static str] },
    /// On or off, kept by the area itself: `get` reads it, `set` changes it (through a command).
    Switch { get: fn(&Arc<Editor>) -> bool, set: fn(&Arc<Editor>, bool), default: bool },
    /// A value to read, never edited: data (`Tx::raw`, a folder, a version) or the area's own
    /// words.
    ReadOnly { value: fn(&Arc<Editor>) -> gz_ui::Tx<'static> },
    /// A button and, beside it, the sentence `status` answers (what the last run found). `run`
    /// starts the work (a command); `enabled` says why the button is off.
    Button {
        text: &'static str,
        run: fn(&Arc<Editor>),
        enabled: fn(&Arc<Editor>) -> crate::actions::Enabled,
        status: fn(&Arc<Editor>) -> gz_ui::Tx<'static>,
    },
    /// A state that is in order or is not: in order (`done`), a check mark and the sentence
    /// `status` answers, and no button to press; otherwise the button that puts it in order
    /// (`run`, off with `enabled`'s reason) beside the sentence.
    Fix {
        text: &'static str,
        run: fn(&Arc<Editor>),
        enabled: fn(&Arc<Editor>) -> crate::actions::Enabled,
        status: fn(&Arc<Editor>) -> gz_ui::Tx<'static>,
        done: fn(&Arc<Editor>) -> bool,
    },
}

impl PrefRow {
    /// A row editing a setting of the editor's settings file.
    #[must_use]
    pub const fn setting(def: crate::settings::PrefDef, label: &'static str, help: &'static str) -> Self {
        Self { label, help, kind: PrefKind::Setting { def, unit: "", choices: &[] } }
    }

    /// The setting this row edits, if it edits one of the settings file.
    #[must_use]
    pub fn def(&self) -> Option<crate::settings::PrefDef> {
        match self.kind {
            PrefKind::Setting { def, .. } => Some(def),
            _ => None,
        }
    }
}

/// Everything the areas plugged in, consumed by the shell when the window opens.
#[derive(Default)]
pub struct ShellBuilder {
    pub(crate) factories: FxHashMap<SmolStr, Factory>,
    pub(crate) actions: Vec<Action>,
    pub(crate) pref_groups: Vec<PrefGroup>,
    pub(crate) menus: Vec<MenuEntry>,
    pub(crate) project_toolbar: Vec<(i32, ToolbarFn)>,
    pub(crate) drawers: [Option<DrawerDesc>; 2],
    pub(crate) status_chips: Vec<(i32, DrawFn)>,
    pub(crate) start_page: Option<DrawFn>,
    pub(crate) page_cards: Vec<PageCard>,
    pub(crate) dialogs: Vec<DialogDesc>,
    pub(crate) panels: Vec<PanelDesc>,
    pub(crate) log_state: Option<LogStateFn>,
    pub(crate) reset_layout: Option<ResetLayoutFn>,
    /// The areas' catalogs of the window's languages (`ru.lua`, Lua data).
    pub(crate) translations: Vec<&'static str>,
}

impl ShellBuilder {
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

    /// The factory of a document kind. A kind registered twice keeps the later factory.
    pub fn document(
        &mut self,
        kind: &str,
        f: impl FnMut(&std::sync::Arc<Editor>, &crate::OpenRequest) -> Result<Box<dyn crate::Document>, String>
        + 'static,
    ) {
        self.factories.insert(SmolStr::new(kind), Box::new(f));
    }

    /// An action: what a menu row, a toolbar button, a shortcut and a palette row run. An id
    /// registered twice keeps the later action.
    pub fn action(&mut self, action: Action) {
        self.actions.retain(|a| a.id != action.id);
        self.actions.push(action);
    }

    /// Whether an action of this id was added (a test of an application's own builder).
    #[must_use]
    pub fn has_action(&self, id: &str) -> bool {
        self.actions.iter().any(|a| a.id == id)
    }

    /// A menu row running an action; `label` overrides its title ("Package Release...").
    pub fn menu_action(&mut self, menu: &str, group: i32, order: i32, id: &str, label: Option<&str>) {
        self.menus.push(MenuEntry {
            menu: SmolStr::new(menu),
            submenu: None,
            group,
            order,
            content: MenuContent::Action { id: SmolStr::new(id), label: label.map(SmolStr::new) },
        });
    }

    /// A menu row running an action, in a submenu of `menu`.
    pub fn submenu_action(
        &mut self,
        menu: &str,
        submenu: &str,
        group: i32,
        order: i32,
        id: &str,
        label: Option<&str>,
    ) {
        self.menus.push(MenuEntry {
            menu: SmolStr::new(menu),
            submenu: Some(SmolStr::new(submenu)),
            group,
            order,
            content: MenuContent::Action { id: SmolStr::new(id), label: label.map(SmolStr::new) },
        });
    }

    /// Rows an area adds to Preferences.
    pub fn pref_group(&mut self, group: PrefGroup) {
        self.pref_groups.push(group);
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

    /// An area's catalog of a language: the Lua data file it embeds (`include_str!("ru.lua")`).
    /// The shell reads every one the first time the window is to speak its language.
    pub fn translation(&mut self, source: &'static str) {
        self.translations.push(source);
    }

    /// The catalogs added, for a test of an application's own builder.
    #[must_use]
    pub fn translations(&self) -> &[&'static str] {
        &self.translations
    }
    /// Reset Layout: `true` also forgets every split and grid width.
    pub fn reset_layout(&mut self, f: impl FnMut(bool, &mut DocCx<'_>) + 'static) {
        self.reset_layout = Some(Box::new(f));
    }
}
