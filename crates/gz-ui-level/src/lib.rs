//! The Level tab: tab 1 of the editor, the dock layout around the viewport and the panels in it
//! (docs/UI_DESIGN.md, sections 5 "Level workspace" / "Level toolbar" / "Status bar" and 6
//! "World Outliner", "Tools", "Details and Level Settings").
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
//!   revision and the filter, laid out only where they show), Tools ([`tools`]: tiles, the
//!   active tool's form, Snap Objects; the Detail Objects dialog [`detail_objects`]), Details and
//!   Level Settings ([`details`]: the kit's property grid over `gz-scene`'s rows, choosers
//!   `###dlg.choose` [`choose`], the text dialog `###dlg.edit_text`), and the hosts of the
//!   Content Browser and the Output Log (`gz-ui-content`, `gz-ui-app`).
//! - **Status bar**: the centre ([`status`]) and the Rebuild chip.
//! - **Scene menus** ([`menus`]): Scene, Build, View on the Level tab only, with the keys bound
//!   now; their commands are [`gz_app::UiCommand`]s, so the palette and the Keyboard page know
//!   them.
//! - **Recovery** ([`recovery`]): the M page card Recover Unsaved Scene; **Import Base Scene**
//!   ([`import`]): the M dialog `dlg.import_base_scene`.
//!
//! [`install`] plugs all of it into the shell; [`register`] adds the commands `outliner_show`,
//! `outliner_filter`, `scene_visibility` and the panel side of `property_inspector` (it keeps
//! `gz-scene`'s answer and adds `open` / `close` / `expand` / `collapse` of the panels).
//!
//! Every action is a command: a `SceneCmd` of `gz-scene`, a command of the registry
//! (`save_scene`, `undo`, `mark_authored`, `recovery`, the level builds of `gz-levelbuild` when
//! that crate registers them - [`toolbar::LEVEL_ACTIONS`]), or `gz-project`'s import. The panels
//! read snapshots and queries; a frame never computes and never touches a file.
//!
//! Widgets the kit lacks, local to this crate ([`local`]): `local_eye_toggle`,
//! `local_button_flow`, `local_panel_header`, `local_tool_tile`, `local_thumbnail`,
//! `local_badge_button`, `local_tab` (a panel tab with its cross on hover), `local_tree_list`
//! (the filtered tree list of a tool).

#![forbid(unsafe_code)]

pub mod choose;
pub mod classes;
mod cmd;
pub mod content;
pub mod detail_objects;
pub mod details;
pub mod dock;
pub mod doc;
pub mod import;
pub mod layout;
pub mod lists;
pub mod local;
pub mod menus;
pub mod outliner;
pub mod outliner_model;
pub mod recovery;
pub mod status;
pub mod toolbar;
pub mod tools;

use gz_app::{PanelDesc, ShellBuilder};
use gz_editor::{Registry, SmolStr};
use gz_ui::Icon;

pub use doc::LevelDoc;

/// The crate's name, as the registry records who registered a command.
const AREA: &str = "gz-ui-level";

/// The tab id of the Level document.
pub const LEVEL: &str = "level";

/// Plugs the Level document, its panels, the scene menus' commands, the Rebuild chip, the
/// Recovery card and the Import Base Scene dialog into the shell.
pub fn install(b: &mut ShellBuilder) {
    b.document(LEVEL, |editor, _req| Ok(Box::new(LevelDoc::new(editor)) as Box<dyn gz_app::Document>));
    for (id, label, icon) in [
        (layout::PanelId::Tools.id(), "Tools", Icon::Settings),
        (layout::PanelId::Outliner.id(), "World Outliner", Icon::Level),
        (layout::PanelId::Details.id(), "Details", Icon::Info),
        (layout::PanelId::LevelSettings.id(), "Level Settings", Icon::Level),
    ] {
        b.panel(PanelDesc { id: SmolStr::new(id), label: SmolStr::new(label), icon, drawer: None });
    }
    b.panel(PanelDesc { id: SmolStr::new("viewport"), label: SmolStr::new("Viewport"), icon: Icon::Perspective, drawer: None });
    menus::install(b);
    status::install(b);
    recovery::install(b);
    import::install(b);
    b.reset_layout(|all, cx| doc::post_to_level(cx.editor, doc::Act::ResetLayout(all)));
}

/// The commands of this package: `outliner_show`, `outliner_filter`, `scene_visibility` and
/// `property_inspector` (registered after `gz-scene`'s, which it wraps).
pub fn register(reg: &mut Registry) {
    cmd::register(reg);
}
