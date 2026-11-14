//! The Level tab: tab 1 of the editor, the dock layout around the viewport and the panels in it
//! (docs/UI_DESIGN.md, sections 5 "Level workspace" / "Level toolbar" / "Status bar" and 6
//! "World Outliner", "The left panel: Place, Paint, Zones", "Details and Level Settings").
//!
//! - **The Level document** ([`doc::LevelDoc`], kind `level`, never closes): the Level toolbar
//!   ([`toolbar`]) over a dock of its own ([`dock`], [`layout`]) with the viewport
//!   (`gz_ui_viewport::ViewportPane`) in the centre. Side panels keep their pt size and the
//!   viewport takes every resize; panel tabs sit on a 26 pt strip with their cross on hover;
//!   nothing leaves the window. The two size classes (Wide, Compact) build their layouts;
//!   crossing a class rebuilds unless the author customised it; Reset Layout rebuilds for the
//!   current class; F11 hides the panels. The layout lives in the kit's layout store (`level.*`
//!   keys), which the editor's settings file keeps.
//! - **Panels**: the World Outliner ([`outliner`], rows from a `Query` keyed by the scene's
//!   revision and the filter, laid out only where they show), the left panel ([`tools`]: the
//!   Place panel [`place`] in Select and Place mode, the AI Map, Foliage and Decals panels of
//!   Paint mode in `tools_bulk` with the Colour Indices dialog [`detail_objects`], the Zones
//!   panel in `tools_objects`), Details and
//!   Level Settings ([`details`]: the kit's property grid over `gz-scene`'s rows, choosers
//!   `###dlg.choose` [`choose`], the text dialog `###dlg.edit_text`), and the hosts of the
//!   Content Browser and the Output Log (`gz-ui-content`, `gz-ui-app`).
//! - **Status bar**: the centre ([`status`]) and the Build chip with its popover of the steps.
//! - **Actions** ([`actions`]): every menu row, toolbar button and key of the tab is an action
//!   over a command (`scene.*`, `level.*`, `level.show`), so the palette, the Keyboard page and an
//!   agent reach the same code. The Level toolbar ([`toolbar`]: Save, Build Level, Play, Stop,
//!   More), the caption's Scene, Build and View menus ([`menus`]), the File menu's scene rows,
//!   the file dialogs and the Summary Info window ([`level_cmds`]), the Problems panel beside the
//!   Output Log ([`problems`]), and the chooser's pictures ([`choose_pics`]).
//! - **Recovery** ([`recovery`]): the M page card Recover Unsaved Scene; **Import Base Scene**
//!   ([`import`]): the M dialog `dlg.import_base_scene`.
//!
//! [`install`] plugs all of it into the shell; [`register`] adds `scene.choices` (with the lists
//! this crate reads), `scene.choose`, `level.show` and `level.view`.
//!
//! The panels read snapshots and queries; a frame never computes and never touches a file.
//!
//! Widgets the kit lacks, local to this crate ([`local`]): `local_eye_toggle`,
//! `local_lock_toggle`, `local_button_flow`, `local_panel_header`, `local_thumbnail`,
//! `local_badge_button`, `local_tree_list` (the filtered tree list of a chooser);
//! `tools_bulk`'s `local_number_row`. The panel tabs are the kit's `gz_ui::tabs`.

#![forbid(unsafe_code)]

pub mod actions;
pub mod choose;
pub mod choose_pics;
pub mod classes;
mod cmd;
pub mod content;
pub mod detail_objects;
pub mod details;
pub mod doc;
pub mod dock;
pub mod import;
pub mod layout;
pub mod level_cmds;
pub mod lists;
pub mod local;
pub mod menus;
mod motion_ui;
pub mod outliner;
pub mod outliner_model;
pub mod place;
pub mod problems;
pub mod recovery;
pub mod status;
pub mod toolbar;
pub mod tools;
mod tools_bulk;
mod tools_objects;
mod view_cmd;

use gz_app::{PanelDesc, ShellBuilder};
use gz_editor::{Registry, SmolStr};
use gz_ui::Icon;

pub use doc::LevelDoc;

/// The tab id of the Level document.
pub const LEVEL: &str = "level";

/// Plugs the Level document, its panels, its actions and their menu rows, the Build chip, the
/// Recovery card and the Import Base Scene dialog into the shell.
pub fn install(b: &mut ShellBuilder) {
    b.document(LEVEL, |editor, _req| Ok(Box::new(LevelDoc::new(editor)) as Box<dyn gz_app::Document>));
    for (id, label, icon) in [
        // one icon each: the Window menu tells them apart by it
        (layout::PanelId::Place.id(), "Place", Icon::Place),
        (layout::PanelId::Outliner.id(), "World Outliner", Icon::SceneObject),
        (layout::PanelId::Details.id(), "Details", Icon::Info),
        (layout::PanelId::LevelSettings.id(), "Level Settings", Icon::Level),
        (layout::PanelId::Problems.id(), "Problems", Icon::Problems),
    ] {
        b.panel(PanelDesc { id: SmolStr::new(id), label: SmolStr::new(label), icon, drawer: None });
    }
    b.panel(PanelDesc {
        id: SmolStr::new("viewport"),
        label: SmolStr::new("Viewport"),
        icon: Icon::Perspective,
        drawer: None,
    });
    actions::install(b);
    level_cmds::install(b);
    status::install(b);
    recovery::install(b);
    import::install(b);
    b.reset_layout(|all, cx| doc::post_to_level(cx.editor, doc::Act::ResetLayout(all)));
}

/// The commands of this package: `scene.choices` (registered after `gz-scene`'s, which it
/// wraps), `scene.choose`, `level.show` and `level.view`.
pub fn register(reg: &mut Registry) {
    cmd::register(reg);
    view_cmd::register(reg);
}
