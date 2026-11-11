//! The Content Browser (docs/UI_DESIGN.md, section 6, "Content Browser"): the project's files,
//! the editor's library and the linked game's content as folders and tiles.
//!
//! **One browser, two hosts.** The same state ([`ContentBrowser`], an `Editor` service) draws as
//! the Level tab's docked panel and as the Content Drawer over any tab, one host per frame; each
//! host keeps its own tile size and view (tiles or list). The drawer's body is plugged in by
//! [`install`]; the Level document draws the panel by calling [`content_browser`] with
//! [`Host::Panel`].
//!
//! - **Sources** ([`model::Source`]): Project (the open project's file tree, `gz-project`'s
//!   snapshot, kept current by its watcher), Editor (the editor's own library, one category at a
//!   time: each category is a folder of the editor's data root, walked once with `gz-project`'s
//!   walker) and Game (the linked game's index, `gz-vfs`). A source's listing is a
//!   [`catalog::Catalog`] built on the pools; a folder's entries, sorted and filtered, are a
//!   `gz_query::Query` keyed by (source, folder, search) and the revisions they read. A frame
//!   never lists, decodes or touches a file.
//! - **Pictures arrive later** ([`pictures`]): a texture's tile picture
//!   (`gz_fmt_tex::texture_tile`, made for the size it is drawn at) and a model's
//!   (`gz_render::ThumbnailCache`, rendered offscreen by a renderer of this crate's own) are
//!   asked for by key and drawn when they are there; until then the vector glyph. Decoding is
//!   bounded and dropped when the folder changes; textures go a few frames after the last one
//!   that drew them.
//! - **Captions** ([`caption`]): `DrawTileCaption` as a pure function of the name and measured
//!   widths.
//! - **Everything a person can do is a command** ([`actions`]): rename, move (drag and drop),
//!   copy, delete (after the kit's confirmation, which names what goes), new folder, Add,
//!   Exclude from Build / Include in Build, open, reveal - the browser emits `editor.call(..)` /
//!   `editor.post(..)` and changes no file itself.
//! - **Drag source**: a tile drags a [`DragPayload`] (egui's drag-and-drop payload); the Screens
//!   tab and the viewport may accept it.
//!
//! [`register`] adds the browser's commands: `content_browser_navigation`,
//! `content_browser_open`, `content_browser_selection`, `content_browser_copy`. The browser's
//! Add goes through `gz-project`'s `content_create` and names the folder it shows.
//!
//! Widgets the kit lacks, local to this crate ([`local`]): `local_tree_row` (a row of the folder
//! tree), `local_section` (a folding header outside a property grid), `local_breadcrumbs` (the
//! folder path whose leading parts fold into "…"), `local_rename_box` (the inline rename field
//! with its own selection), `local_checker` (the checkerboard under a picture with transparency).
//!
//! The C++ specification is `UIContentBrowser.cpp`, `EThumbnailTexture.cpp`,
//! `EThumbnailVisual.cpp`, `EditorAssetIcons.cpp` and the handlers in `LevelMcpContent.cpp`.

#![forbid(unsafe_code)]

pub mod actions;
pub mod caption;
pub mod catalog;
mod cmd;
mod draw;
pub mod icons;
pub mod local;
pub mod model;
pub mod pictures;
pub mod select;
mod service;
mod thumbs;

use gz_app::{Drawer, DrawerDesc, PanelDesc, ShellBuilder};
use gz_editor::{Registry, SmolStr};
use gz_ui::Icon;

pub use draw::content_browser;
pub use model::{Host, Source};
pub use service::ContentBrowser;

/// The crate's name, as the registry records who registered a command.
const AREA: &str = "gz-ui-content";

/// The Level panel's id and the window it is drawn in (`EditorUiContract`).
pub const PANEL_ID: &str = "content_browser";

/// What a tile drags: the entries it carries, the selection when the dragged tile is selected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DragPayload {
    pub source: Source,
    pub entries: Vec<DraggedEntry>,
}

/// One dragged entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DraggedEntry {
    /// As the source names it: project-relative, Editor-category-relative or `gamedata`-relative,
    /// backslashes.
    pub path: String,
    pub folder: bool,
    /// Where it is on disk, when it is a file of its own (a game file in an archive is not).
    pub file: Option<std::path::PathBuf>,
}

/// Plugs the browser into the shell: the Level panel's description, the Content Drawer's body
/// and the tab menu's Show in Content Browser.
pub fn install(b: &mut ShellBuilder) {
    b.panel(PanelDesc {
        id: SmolStr::new(PANEL_ID),
        label: SmolStr::new("Content Browser"),
        icon: Icon::Folder,
        drawer: Some(Drawer::Content),
    });
    b.drawer(
        Drawer::Content,
        DrawerDesc {
            label: SmolStr::new("Content Browser"),
            icon: Icon::Folder,
            panel: Some(SmolStr::new(PANEL_ID)),
            body: Box::new(|ui: &mut egui::Ui, cx: &mut gz_app::DocCx<'_>| {
                content_browser(ui, cx, Host::Drawer)
            }),
        },
    );
    b.reveal_in_browser(|path, cx| actions::reveal_from_tab(cx, path));
}

/// What the browser tells the rest of the editor: the folder `content_create` makes an asset in
/// when the caller names none (the folder shown, as the C++ `UIContentBrowser::GetSelection`).
pub fn install_hooks(editor: &gz_editor::Editor) {
    gz_project::ProjectHooks::of(editor).provide_content_folder(|ed| {
        let loc = service::ContentBrowser::of(ed).inner.lock().loc.clone();
        match loc.source {
            model::Source::Project => Ok(loc.folder.replace('/', "\\")),
            model::Source::Editor | model::Source::Game => {
                Err("only the project source can be written to".to_owned())
            }
        }
    });
}

/// The commands of this package.
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "content_browser_navigation", cmd::content_browser_navigation);
    reg.add(AREA, "content_browser_open", cmd::content_browser_open);
    reg.add(AREA, "content_browser_selection", cmd::content_browser_selection);
    reg.add(AREA, "content_browser_copy", cmd::content_browser_copy);
}
