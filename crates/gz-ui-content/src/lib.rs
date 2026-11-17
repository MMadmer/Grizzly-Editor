//! The Content Browser (docs/UI_DESIGN.md, section 6, "Content Browser"): the project's files
//! and the Library - what can be brought into the project or placed - as folders and tiles.
//!
//! **One browser, two hosts.** The same state ([`ContentBrowser`], an `Editor` service) draws as
//! the Level tab's docked panel and as the Content Drawer over any tab, one host per frame; each
//! host keeps its own tile size and view (tiles or list). The drawer's body is plugged in by
//! [`install`]; the Level document draws the panel by calling [`content_browser`] with
//! [`Host::Panel`].
//!
//! - **Sources** ([`model::Source`]): Project (the open project's file tree, `gz-project`'s
//!   snapshot, kept current by its watcher) and Library ([`library`]): the linked game and the
//!   editor's library merged, by kind - Objects, Groups, Textures, Sounds, Particles, Spawn
//!   Sections, Base Scenes. A source's listing is a [`catalog::Catalog`] built on the pools; a
//!   folder's entries, sorted and filtered, are a `gz_query::Query` keyed by (source, folder,
//!   search) and the revisions they read. A frame never lists, decodes or touches a file.
//! - **One tile per asset** ([`catalog`]): a `.thm`, a backup, a scene's folder are never tiles;
//!   they go with their asset.
//! - **Pictures arrive later** ([`pictures`]): a texture's tile picture (made for the size it is
//!   drawn at), an object's or a group's `.thm` thumbnail, a model rendered offscreen by a renderer
//!   of this crate's own when it has none; uploaded within a budget per frame. Until then the
//!   vector glyph.
//! - **Captions** ([`caption`]): a pure function of the name and measured widths.
//! - **Everything a person can do is a command** ([`actions`]): rename, move (drag and drop),
//!   copy, delete (after the kit's confirmation, which names what goes), new folder, Add,
//!   Exclude from Build / Include in Build, Bring into Project, Place in Level, open, reveal - the
//!   browser runs `file.*`, `scene.*` and its own `content.view`, and changes no file itself.
//! - **Drag source** ([`payload_of`]): a tile drags the kit's `gz_ui::drop::Payload::Assets`;
//!   the viewport places what it can, a field takes the kind it holds, a project folder of the
//!   browser moves or imports.
//!
//! [`register`] adds `content.view`: what the browser shows, to look at and to drive.
//!
//! Widgets the kit lacks, local to this crate ([`local`]): `local_tree_row` (a row of the folder
//! tree), `local_breadcrumbs` (the folder path whose leading parts fold into "…"),
//! `local_rename_box` (the inline rename field with its own selection), `local_marquee`,
//! `local_menu_heading`. The checkerboard under a picture with transparency is the kit's
//! `gz_ui::checker`.

#![forbid(unsafe_code)]

pub mod actions;
pub mod caption;
pub mod catalog;
mod cmd;
mod draw;
pub mod icons;
pub mod library;
pub mod local;
pub mod model;
pub mod pictures;
pub mod select;
mod service;
mod thumbs;

use gz_app::{Drawer, DrawerDesc, PanelDesc, ShellBuilder};
use gz_editor::{Registry, SmolStr};
use gz_ui::Icon;
use gz_ui::drop::Asset;

pub use draw::content_browser;
pub use model::{Host, Source};
pub use service::ContentBrowser;

/// The Level panel's id and the window it is drawn in.
pub const PANEL_ID: &str = "content_browser";

/// What a dragged entry is to whoever takes it: the kit's [`Asset`].
///
/// - `kind`: for what is placed in a level, the `scene.place` kind - `object` (a model asset
///   too), `group`, `sound` (a sound asset too), `particles`, `spawn`; otherwise `texture` (a
///   texture asset too), `model`, `scene`, `folder`, one of the editor's own kinds (`quest`,
///   `behaviour`, `character`, `character_template`, `pda`) or `file`.
/// - `path`: an object or a group by its file - below the library's `rawdata\objects` /
///   `rawdata\groups`, or project-relative - with its extension (a model asset by its project
///   path); a texture or a sound asset by the name it publishes; a sound or a particle effect by its
///   reference (`ambient\wind`); a spawn by its section; a texture by its name below `textures`
///   without the extension; anything else by its path in its source (project-relative,
///   backslashes).
/// - `file`: where it is on disk, when it is a file of its own there.
#[must_use]
pub fn payload_of(cat: &catalog::Catalog, path: &str, folder: bool) -> Asset {
    let file =
        if folder { cat.disk_path(path) } else { cat.disk_path(path).filter(|_| cat.file(path).is_some()) };
    if folder {
        return Asset { path: path.to_owned(), kind: "folder".to_owned(), file };
    }
    let stem = catalog::stem(path).to_owned();
    let item = cat.file(path);
    let (kind, reference): (&str, String) = match cat.source {
        Source::Library => match model::kind(cat.category) {
            Some(k) if cat.category == model::SPAWNS => (k.payload, model::leaf_of(path).to_owned()),
            Some(k) if matches!(cat.category, model::SOUNDS | model::PARTICLES | model::TEXTURES) => {
                (k.payload, stem)
            }
            Some(k) => (k.payload, path.to_owned()),
            None => ("file", path.to_owned()),
        },
        Source::Project => {
            let lower = path.to_ascii_lowercase();
            let ext = icons::extension(&lower).unwrap_or("").to_owned();
            if let Some(kind) = item.and_then(|f| f.asset) {
                use gz_project::AssetKind as K;
                // a texture and a sound go by the name the game knows them by
                let published = || item.and_then(|f| f.published.as_deref()).unwrap_or(&stem).to_owned();
                match kind {
                    K::Quest => ("quest", path.to_owned()),
                    K::Behaviour => ("behaviour", path.to_owned()),
                    K::Character => ("character", path.to_owned()),
                    K::CharacterTemplate => ("character_template", path.to_owned()),
                    K::Pda => ("pda", path.to_owned()),
                    K::Item => ("item", path.to_owned()),
                    K::Trade => ("trader", path.to_owned()),
                    K::Loot => ("loot", path.to_owned()),
                    K::Recipes => ("recipes", path.to_owned()),
                    // a scene object names a model asset by its project path
                    K::Model => ("object", path.to_owned()),
                    K::Texture => ("texture", published()),
                    K::Sound => ("sound", published()),
                    K::Project => ("project", path.to_owned()),
                }
            } else if ext == ".object" && lower.starts_with("rawdata\\objects\\") {
                ("object", path.to_owned())
            } else if ext == ".group" {
                ("group", path.to_owned())
            } else if icons::is_image(path) {
                let name = lower.strip_prefix("gamedata\\textures\\").map_or(stem.clone(), |_| {
                    stem.get("gamedata\\textures\\".len()..).unwrap_or(&stem).to_owned()
                });
                ("texture", name)
            } else if ext == ".ogf" || ext == ".object" {
                ("model", path.to_owned())
            } else if ext == ".scene" || ext == ".level" {
                ("scene", path.to_owned())
            } else if ext == ".ogg" && lower.starts_with("gamedata\\sounds\\") {
                ("sound", stem.get("gamedata\\sounds\\".len()..).unwrap_or(&stem).to_owned())
            } else {
                ("file", path.to_owned())
            }
        }
    };
    Asset { path: reference, kind: kind.to_owned(), file }
}

/// The browser's words in Russian (docs/UI_DESIGN.md, section 9).
pub const RU: &str = include_str!("ru.lua");

/// Plugs the browser into the shell: the Level panel's description, the Content Drawer's body
/// and the tab menu's Show in Content Browser.
pub fn install(b: &mut ShellBuilder) {
    b.translation(RU);
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
}

/// What the browser tells the rest of the editor: the folder `file.create` makes an asset in when
/// the caller names none (the project folder shown); and, while a project is open, that a texture
/// asset or its picture changed on the disk ([`follow_textures`]).
pub fn install_hooks(editor: &gz_editor::Editor) {
    gz_project::ProjectHooks::of(editor).on_open(|ed, _session| follow_textures(ed));
    gz_project::ProjectHooks::of(editor).provide_content_folder(|ed| {
        let loc = service::ContentBrowser::of(ed).inner.lock().loc.clone();
        match loc.source {
            model::Source::Project => Ok(loc.folder.replace('/', "\\")),
            model::Source::Library => Err("only the project source can be written to".to_owned()),
        }
    });
}

/// Whether a change of the project's files is one the window's textures follow: a texture asset,
/// or a file a texture asset takes its picture from.
#[must_use]
pub fn changes_a_texture(path: &str, index: Option<&gz_media::Index>) -> bool {
    use gz_project::asset_kind::MediaKind;
    MediaKind::of_file(path) == Some(MediaKind::Texture)
        || index.is_some_and(|i| i.users_of(path).iter().any(|e| e.kind == MediaKind::Texture))
}

/// A texture asset or its picture changed on the disk: every texture cache of the window - the
/// viewport's renderer, the previews, the tiles - reads its textures again, as Libraries > Reload
/// Textures has them do. The project's watcher says so; nothing polls, and the UI thread does
/// none of it.
fn follow_textures(editor: &std::sync::Arc<gz_editor::Editor>) {
    let Ok((_, project)) = gz_project::Project::of(editor) else { return };
    let mut rx = project.tree.changes().subscribe(gz_project::Filter::All);
    let ed = std::sync::Arc::downgrade(editor);
    editor.exec.spawn(async move {
        while let Some(change) = rx.recv().await {
            let Some(ed) = ed.upgrade() else { break };
            let index = gz_media::Media::get(&ed).and_then(|m| m.latest());
            if changes_a_texture(change.path.as_str(), index.as_deref()) {
                gz_app::gpu::TextureReload::bump(&ed);
            }
        }
    });
}

/// The commands of this package: `content.view`.
pub fn register(reg: &mut Registry) {
    cmd::register(reg);
}

#[cfg(test)]
mod tests {
    #[test]
    fn what_the_window_textures_follow() {
        use gz_project::asset_kind::MediaKind;
        assert!(super::changes_a_texture("textures/Brick.TEXASSET", None));
        assert!(!super::changes_a_texture("textures/brick.png", None));
        assert!(!super::changes_a_texture("models/wall.meshasset", None));
        let rel = |p: &str| gz_base::RelPath::new(p).unwrap();
        let index = gz_media::Index {
            stamp: gz_base::Rev(1),
            module: String::new(),
            entries: vec![gz_media::Entry {
                kind: MediaKind::Texture,
                path: rel("textures/brick.texasset"),
                name: "brick".to_owned(),
                key: "brick".to_owned(),
                source: Some(rel("textures/brick.png")),
                uses: Vec::new(),
                dynamic: false,
                parts: false,
                excluded: false,
                error: None,
            }],
        };
        // the picture a texture asset wears, and nothing else of the folder
        assert!(super::changes_a_texture("textures/BRICK.png", Some(&index)));
        assert!(!super::changes_a_texture("textures/other.png", Some(&index)));
    }

    #[test]
    fn the_catalog_reads_with_the_kits_and_the_shells() {
        let c = gz_ui::i18n::Catalog::parse(&[gz_ui::i18n::KIT_RU, gz_app::lang::SHELL_RU, super::RU])
            .expect("the catalogs read");
        let p = gz_ui::i18n::problems(&c);
        assert!(p.is_empty(), "{}", p.join("\n"));
    }
}
