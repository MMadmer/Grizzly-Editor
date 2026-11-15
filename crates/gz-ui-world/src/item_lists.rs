//! What the Items tab picks from that the Characters tab's lists do not hold, read on the pools
//! and only looked at by a frame: the linked game's items with their families (the + menu's
//! choosers), its traders' configs (Sold By), the items that have a use scene (Use Scene, and
//! from them the families that are used), and the pictures an icon shows.
//!
//! [`ItemLists`] lives as long as the open project. A frame asks [`ItemLists::want`] - a
//! comparison of one revision - and reads [`ItemLists::view`]; when the linked game moved, one
//! task reads the lists again. [`Pictures`] decodes an icon's picture file the first time it is
//! shown, uploads it once and keeps it while the project is open.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{ColorImage, TextureHandle, TextureId, TextureOptions};
use gz_base::Rev;
use gz_editor::Editor;
use gz_game::GameLink;
use gz_items::Items;
use gz_nq::{NqService, PickerEntry, PickerType};
use gz_project::Project;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::call::{post_ui, wake_ui};
use crate::lists::Indexed;

/// The game's file of the use scenes.
const SCENES: &str = "configs\\items\\items\\animations_settings.ltx";
/// Where the traders' configs are.
const TRADERS: &str = "configs/misc/trade";
/// The largest picture read for an icon.
const PICTURE_READ_LIMIT: u64 = 64 << 20;

/// An item of the linked game, as the choosers offer it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GameItem {
    pub section: String,
    /// What the player reads; empty when the game names it nowhere.
    pub name: String,
    pub family: String,
    /// The catalog's title of the family; empty for an item of no family.
    pub family_title: String,
}

/// What the Items tab picks from now.
#[derive(Clone, Default)]
pub struct ItemListsView {
    /// The first read ended: until then a chooser shows busy.
    pub loaded: bool,
    /// By family, then by name.
    pub game_items: Arc<Vec<GameItem>>,
    /// The traders' configs, as `sold.by` names them (`trade_military`).
    pub traders: Arc<Indexed>,
    /// The items that have a use scene, by section.
    pub scenes: Arc<Indexed>,
    /// The families of the items that have a use scene: those a Use Scene is offered for.
    pub scene_families: Arc<Vec<String>>,
    pub serial: u64,
}

impl ItemListsView {
    /// The game's item of a section.
    #[must_use]
    pub fn game_item(&self, section: &str) -> Option<&GameItem> {
        let lc = section.trim().to_ascii_lowercase();
        self.game_items.iter().find(|g| g.section == lc)
    }
}

#[derive(Default)]
struct State {
    view: ItemListsView,
    key: Option<Rev>,
    loading: bool,
    serial: u64,
}

/// The Items tab's lists of the open project (`session.services`).
#[derive(Default)]
pub struct ItemLists {
    state: Mutex<State>,
}

/// The chooser's rows: the items by family title, then by name, the ones of no family last.
fn sorted(mut items: Vec<GameItem>) -> Vec<GameItem> {
    items.sort_by(|a, b| {
        (a.family_title.is_empty(), &a.family_title, a.name.to_lowercase(), &a.section).cmp(&(
            b.family_title.is_empty(),
            &b.family_title,
            b.name.to_lowercase(),
            &b.section,
        ))
    });
    items
}

/// A caption as the player reads it: a string id the game's tables lack is no name.
fn player_name(caption: &str) -> String {
    let c = caption.trim();
    if !c.contains(char::is_whitespace) && c.contains('_') && c.is_ascii() {
        String::new()
    } else {
        c.to_owned()
    }
}

/// The sections of the game's use scenes. Blocking.
fn scene_sections(text: &str) -> Vec<String> {
    let doc = gz_ltx::LtxDoc::parse(text);
    let mut out: Vec<String> =
        doc.sections().map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty()).collect();
    out.sort();
    out.dedup();
    out
}

impl ItemLists {
    #[must_use]
    pub fn of(editor: &Editor) -> Option<Arc<Self>> {
        let session = editor.project_opt()?;
        Some(session.services.get_or_init(Self::default))
    }

    #[must_use]
    pub fn view(&self) -> ItemListsView {
        self.state.lock().view.clone()
    }

    /// Reads the lists again when the linked game moved: a frame calls it, and it only compares
    /// revisions.
    pub fn want(self: &Arc<Self>, editor: &Arc<Editor>) {
        let Ok((_, project)) = Project::of(editor) else { return };
        let key = project.game_rev();
        {
            let mut st = self.state.lock();
            if st.loading || st.key == Some(key) {
                return;
            }
            st.loading = true;
            st.key = Some(key);
        }
        let (ed, me) = (editor.clone(), self.clone());
        let _task = editor.exec.spawn(async move {
            let names: Vec<PickerEntry> = match NqService::of(&ed) {
                Ok(nq) => nq.pickers(&ed).await.entries(PickerType::Item).to_vec(),
                Err(_) => Vec::new(),
            };
            let (game, catalog) = match Items::of(&ed).await {
                Ok(items) => (Some(items.roster(&ed).await.game.clone()), Some(items.catalog(&ed).await)),
                Err(_) => (None, None),
            };
            let files = match GameLink::of(&ed) {
                Ok(link) if Project::of(&ed).is_ok_and(|(_, p)| !p.game_root().is_empty()) => {
                    link.files().await.ok()
                }
                _ => None,
            };
            let serial = {
                let mut st = me.state.lock();
                st.serial += 1;
                st.serial
            };
            let view = ed
                .exec
                .io(move || {
                    let family_of = |section: &str| -> (String, String) {
                        let class = game
                            .as_ref()
                            .and_then(|g| g.section(section))
                            .and_then(|s| s.get("class"))
                            .and_then(|it| it.value.as_deref())
                            .map(str::trim)
                            .unwrap_or_default()
                            .to_owned();
                        match catalog.as_ref().and_then(|c| c.catalog.family_of_class(&class)) {
                            Some(f) => (f.id.clone(), f.title.clone()),
                            None => (String::new(), String::new()),
                        }
                    };
                    let items: Vec<GameItem> = names
                        .iter()
                        .map(|e| {
                            let (family, family_title) = family_of(&e.id);
                            GameItem {
                                section: e.id.to_ascii_lowercase(),
                                name: player_name(&e.name),
                                family,
                                family_title,
                            }
                        })
                        .collect();
                    let mut traders: Vec<PickerEntry> = Vec::new();
                    let mut scenes: Vec<PickerEntry> = Vec::new();
                    if let Some(files) = files.as_deref() {
                        let prefix = format!("{TRADERS}/");
                        for f in files.find(TRADERS, "ltx") {
                            let rel = f.path.to_ascii_lowercase();
                            let Some(rest) = rel.strip_prefix(&prefix).and_then(|r| r.strip_suffix(".ltx"))
                            else {
                                continue;
                            };
                            traders.push(PickerEntry {
                                id: rest.replace('/', "\\"),
                                name: String::new(),
                                extra: String::new(),
                            });
                        }
                        if let Ok(bytes) = files.read(SCENES) {
                            for s in scene_sections(&gz_base::text::decode_guess(&bytes)) {
                                let name = names
                                    .iter()
                                    .find(|e| e.id.eq_ignore_ascii_case(&s))
                                    .map(|e| e.name.clone());
                                scenes.push(PickerEntry {
                                    id: s,
                                    name: name.unwrap_or_default(),
                                    extra: String::new(),
                                });
                            }
                        }
                    }
                    traders.sort_by(|a, b| a.id.cmp(&b.id));
                    let mut families: Vec<String> =
                        scenes.iter().map(|s| family_of(&s.id).0).filter(|f| !f.is_empty()).collect();
                    families.sort();
                    families.dedup();
                    ItemListsView {
                        loaded: true,
                        game_items: Arc::new(sorted(items)),
                        traders: Arc::new(Indexed::new(traders, serial)),
                        scenes: Arc::new(Indexed::new(scenes, serial)),
                        scene_families: Arc::new(families),
                        serial,
                    }
                })
                .await;
            {
                let mut st = me.state.lock();
                st.view = view;
                st.loading = false;
            }
            wake_ui(&ed);
        });
    }

    /// Reads the lists again (Reload).
    pub fn force(self: &Arc<Self>, editor: &Arc<Editor>) {
        {
            let mut st = self.state.lock();
            st.key = None;
            if st.loading {
                return;
            }
        }
        self.want(editor);
    }
}

// ---- the pictures an icon shows ----------------------------------------------------------------

enum Slot {
    Loading,
    Failed,
    Ready { texture: TextureHandle, size: [u32; 2] },
}

/// The pictures of the open project's icons (`session.services`), by file and its write time.
#[derive(Default)]
pub struct Pictures {
    slots: Mutex<FxHashMap<String, Slot>>,
}

/// The pixels of a picture file as egui takes them. Blocking.
fn read_picture(path: &Path) -> Option<ColorImage> {
    if std::fs::metadata(path).ok()?.len() > PICTURE_READ_LIMIT {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let ext = path.extension().and_then(|e| e.to_str());
    let loaded = gz_fmt_tex::load_image(&bytes, ext).ok()?;
    let img = loaded.image;
    Some(ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.rgba))
}

impl Pictures {
    #[must_use]
    pub fn of(editor: &Editor) -> Option<Arc<Self>> {
        let session = editor.project_opt()?;
        Some(session.services.get_or_init(Self::default))
    }

    /// The texture of a picture file and its size in pixels, once it is read; the first ask
    /// starts the read. `stamp` tells one state of the file from another (its write time).
    pub fn get(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        path: &Path,
        stamp: u64,
    ) -> Option<(TextureId, [u32; 2])> {
        let key = format!("{}|{stamp}", path.display());
        let mut slots = self.slots.lock();
        match slots.get(&key) {
            Some(Slot::Ready { texture, size }) => Some((texture.id(), *size)),
            Some(_) => None,
            None => {
                slots.insert(key.clone(), Slot::Loading);
                drop(slots);
                let (ed, me, path) = (editor.clone(), self.clone(), path.to_path_buf());
                let _task = editor.exec.spawn(async move {
                    let image = ed.exec.io(move || read_picture(&path)).await;
                    post_ui(&ed, move |ctx| {
                        let slot = match image {
                            Some(img) => {
                                let size = [img.size[0] as u32, img.size[1] as u32];
                                let texture = ctx.load_texture(
                                    format!("gz-ui-world.icon.{key}"),
                                    img,
                                    TextureOptions::LINEAR,
                                );
                                Slot::Ready { texture, size }
                            }
                            None => Slot::Failed,
                        };
                        me.slots.lock().insert(key, slot);
                    });
                });
                None
            }
        }
    }

    /// Whether the picture of this state of the file did not read.
    #[must_use]
    pub fn failed(&self, path: &Path, stamp: u64) -> bool {
        matches!(self.slots.lock().get(&format!("{}|{stamp}", path.display())), Some(Slot::Failed))
    }
}

/// The file a picture icon of an asset names, on the disk.
#[must_use]
pub fn picture_path(root: &Path, asset: &str, file: &str) -> PathBuf {
    gz_media::files::beside(&root.join(asset.replace('/', "\\")), file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_choosers_rows_go_by_family_then_name() {
        let g = |s: &str, n: &str, f: &str| GameItem {
            section: s.into(),
            name: n.into(),
            family: f.to_lowercase(),
            family_title: f.into(),
        };
        let rows = sorted(vec![
            g("b", "Zed", "Food"),
            g("odd", "Odd", ""),
            g("a", "alpha", "Food"),
            g("w", "AK", "Firearm"),
        ]);
        let order: Vec<&str> = rows.iter().map(|r| r.section.as_str()).collect();
        assert_eq!(order, ["w", "a", "b", "odd"]);
        assert_eq!(scene_sections("[vodka]\nsnd = x\n[Bread]\n[vodka]\n"), ["bread", "vodka"]);
    }
}
