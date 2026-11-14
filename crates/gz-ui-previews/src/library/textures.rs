//! The Texture Library: the `texture_library` tab over the descriptions of `rawdata\textures`, and
//! New Textures - the same tab over the import folder. The library looks at the import folder by
//! itself as it opens and offers what is new there (Import New Textures); Save writes the changed
//! descriptions and builds their game textures as a job beside the editor. The manual work -
//! checking the import folder, synchronizing with the sources, rewriting the descriptions,
//! reloading the textures - is under the tab's More.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Color32, CornerRadius, Rect, Sense, TextureHandle, Ui, Vec2, pos2};
use gz_app::{DocCx, Document, EditAction, OpenRequest};
use gz_editor::{Args, CmdError, Editor, JobId, Origin};
use gz_fmt_tex::{
    BumpMode, MipFilterId, TextureFlags, TextureFormat, TextureMaterial, TextureParams, TextureType,
    read_texture_thm, texture_tile,
};
use gz_store::{Apply, State, Store, imbl};
use gz_ui::grid::{Grid, color, combo, number, read_only, switch};
use gz_ui::text::{self, Clip};
use gz_ui::{Button, Font, Icon, Kind, PropFlags, PropertyGrid, Tone, Toolbar, Tx, palette, state, tr};
use serde_json::{Map, Value, json};

use super::keys::apply_texture;
use super::list::{Empty, LibraryList, find, shown};
use super::stores::{LibraryFacts, LibraryStores, TexFacts};
use super::texpick::{Names, texture_pick};
use super::texsync::{import_picture, read_picture, sync_texture};
use super::{LibraryDoc, LibraryLists, Settle, can_write, folder_text, folder_tx, pane, panes};
use crate::local::local_heading;
use crate::paths::{EditorDirs, list_files, texture_folder_name};
use crate::pending::Pending;
use crate::writer::LibraryWriter;

pub const KIND: &str = "texture_library";
/// The largest side the preview draws, pt.
const IMAGE_MAX: f32 = 512.0;

/// A texture of the list: its key, its description as read, where its picture is.
#[derive(Clone, Debug)]
pub struct TexItem {
    /// The library's key (`wpn\wpn_ak74`), or the import file's name with its extension.
    pub name: String,
    pub params: TextureParams,
    /// The picture the preview shows.
    pub picture: PathBuf,
}

/// The library's list: every `.tga` of `rawdata\textures` with its `.thm`, read in parallel.
#[must_use]
pub fn list_library(dirs: &EditorDirs, pool: &gz_task::Executors) -> Vec<TexItem> {
    let files = list_files(&dirs.textures, &["tga"], false, true);
    let dir = dirs.textures.clone();
    pool.io_pool().install(|| {
        use rayon::prelude::*;
        files
            .into_par_iter()
            .map(|(name, path)| {
                let params = std::fs::read(dir.join(format!("{name}.thm")))
                    .ok()
                    .and_then(|b| read_texture_thm(&b).ok())
                    .map(|t| t.params)
                    .unwrap_or_default();
                TexItem { name, params, picture: path }
            })
            .collect()
    })
}

/// New Textures: the pictures at the import folder's root, each with a
/// description of its own, the one a game texture of its name has, or a new picture's defaults.
#[must_use]
pub fn list_import(dirs: &EditorDirs) -> Vec<TexItem> {
    let files = list_files(&dirs.import, &PICTURE_EXTS, true, false);
    files
        .into_iter()
        .map(|(name, path)| {
            let stem = crate::paths::strip_ext(&name).to_owned();
            let own = std::fs::read(dirs.import.join(format!("{stem}.thm")))
                .ok()
                .and_then(|b| read_texture_thm(&b).ok());
            let game = || {
                std::fs::read(dirs.game_textures.join(format!("{}.thm", texture_folder_name(&stem))))
                    .ok()
                    .and_then(|b| read_texture_thm(&b).ok())
            };
            let mut params = own.or_else(game).map(|t| t.params);
            if let Ok((img, alpha)) = read_picture(&path) {
                let p = params
                    .get_or_insert_with(|| TextureParams::for_new_texture(img.width, img.height, alpha));
                p.width = i32::try_from(img.width).unwrap_or(i32::MAX);
                p.height = i32::try_from(img.height).unwrap_or(i32::MAX);
                p.flags.set(TextureFlags::HAS_ALPHA, alpha);
            }
            TexItem { name, params: params.unwrap_or_default(), picture: path }
        })
        .collect()
}

/// The descriptions edited since the last save.
#[derive(Clone, Default)]
pub struct TexEdits {
    pub edits: imbl::OrdMap<String, TextureParams>,
}

pub enum TexCmd {
    Set {
        name: String,
        params: TextureParams,
        gesture: String,
    },
    /// The edits of these entries dropped: a save of those alone wrote them.
    Drop {
        names: Vec<String>,
    },
    /// Every edit dropped (a revert, a save that wrote them, a new list).
    Reset,
}

impl State for TexEdits {
    type Cmd = TexCmd;
    type Reply = ();

    fn apply(&mut self, cmd: TexCmd, cx: &mut Apply) -> Result<(), CmdError> {
        match cmd {
            TexCmd::Set { name, params, gesture } => {
                self.edits.insert(name, params);
                cx.undoable_merged("Texture Description", gesture);
            }
            TexCmd::Drop { names } => {
                for n in &names {
                    self.edits.remove(n);
                }
                if self.edits.is_empty() {
                    cx.new_document();
                } else {
                    cx.content_changed();
                }
            }
            TexCmd::Reset => {
                self.edits = imbl::OrdMap::new();
                cx.new_document();
            }
        }
        Ok(())
    }
}

/// The picture of the selected texture, made on the pools at most 512 a side.
struct Preview {
    name: String,
    pending: Pending<Option<egui::ColorImage>>,
    texture: Option<TextureHandle>,
}

/// The Texture Library tab.
pub struct TexLibDoc {
    editor: Arc<Editor>,
    dirs: EditorDirs,
    store: Store<TexEdits>,
    /// New Textures: the import folder's pictures.
    pub import: Option<Pending<Vec<TexItem>>>,
    list: LibraryList,
    pub selected: String,
    filter: [bool; 5],
    /// Whether the texture folder can be written: looked at on the IO pool as the tab opens.
    writable: Pending<bool>,
    /// The pictures the import folder holds that the library does not, counted as the tab opens.
    new_pictures: Pending<usize>,
    /// A save is running.
    saving: Arc<std::sync::atomic::AtomicBool>,
    /// What stopped the save a command's caller asked for.
    save_error: super::SaveError,
    preview: Option<Preview>,
}

/// The words `library.view types=` takes, in the order of the type filter.
pub const TYPE_WORDS: [&str; 5] = ["images", "cube_maps", "bump_maps", "normal_maps", "terrain"];

impl TexLibDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>, import: bool) -> Self {
        let dirs = EditorDirs::of(editor);
        let (d, log, exec) = (dirs.textures.clone(), editor.log.clone(), editor.exec.clone());
        let writable = Pending::spawn(&editor.exec, async move {
            let ok = exec.io(move || can_write(&d)).await;
            if !ok {
                log.warn("The texture folder cannot be written: texture descriptions are read-only.");
            }
            ok
        });
        let (d, exec) = (dirs.clone(), editor.exec.clone());
        let new_pictures =
            Pending::spawn(&editor.exec, async move { exec.io(move || list_import(&d).len()).await });
        let stores = LibraryStores::of(editor);
        let mut doc = Self {
            editor: editor.clone(),
            store: stores.tex.clone(),
            dirs,
            import: None,
            list: LibraryList::default(),
            selected: String::new(),
            filter: [true; 5],
            writable,
            new_pictures,
            saving: stores.tex_saving.clone(),
            save_error: Arc::default(),
            preview: None,
        };
        if import {
            doc.start_import();
        }
        doc
    }

    /// Turns the tab into New Textures: the import folder is read on the IO pool.
    pub fn start_import(&mut self) {
        let dirs = self.dirs.clone();
        let exec = self.editor.exec.clone();
        self.import =
            Some(Pending::spawn(&self.editor.exec, async move { exec.io(move || list_import(&dirs)).await }));
        self.selected.clear();
        self.preview = None;
        self.list.search.clear();
        self.store.post(TexCmd::Reset, Origin::Ui);
    }

    /// The library comes back (a finished import, nothing new to import).
    fn back_to_library(&mut self) {
        self.import = None;
        let (d, exec) = (self.dirs.clone(), self.editor.exec.clone());
        self.new_pictures =
            Pending::spawn(&self.editor.exec, async move { exec.io(move || list_import(&d).len()).await });
        self.selected.clear();
        self.preview = None;
        self.list.search.clear();
        self.store.post(TexCmd::Reset, Origin::Ui);
        LibraryLists::of(&self.editor).textures_rev.bump();
    }

    /// The texture folder cannot be written (unknown until the IO pool has looked: writable).
    fn read_only(&self) -> bool {
        self.writable.get().is_some_and(|w| !*w)
    }

    fn import_mode(&self) -> bool {
        self.import.is_some()
    }

    fn saving(&self) -> bool {
        self.saving.load(std::sync::atomic::Ordering::Acquire)
    }

    /// The list on show: the library's (a query per revision) or the import folder's.
    fn items(&self) -> Option<Arc<Vec<TexItem>>> {
        if let Some(p) = &self.import {
            return p.get();
        }
        let lists = LibraryLists::of(&self.editor);
        let (dirs, exec) = (self.dirs.clone(), self.editor.exec.clone());
        lists.textures.get(lists.textures_rev.get(), move || list_library(&dirs, &exec)).value().cloned()
    }

    /// The same, awaited: what a command reads.
    pub async fn items_now(editor: &Editor, import: Option<Pending<Vec<TexItem>>>) -> Arc<Vec<TexItem>> {
        if let Some(p) = import {
            return p.wait(std::time::Duration::from_secs(30)).await.unwrap_or_default();
        }
        let lists = LibraryLists::of(editor);
        let (dirs, exec) = (EditorDirs::of(editor), editor.exec.clone());
        lists.textures.fresh(lists.textures_rev.get(), move || list_library(&dirs, &exec)).await
    }

    fn visible(&self, item: &TexItem) -> bool {
        if self.import_mode() {
            return true;
        }
        let ty = item.params.ty.0 as usize;
        self.filter.get(ty).copied().unwrap_or(true)
    }

    /// The description on show: the edit when there is one.
    fn params_of(&self, item: &TexItem) -> TextureParams {
        self.store.snapshot().edits.get(&item.name).cloned().unwrap_or_else(|| item.params.clone())
    }

    fn dirty_now(&self) -> bool {
        match &self.import {
            Some(p) => p.get().is_some_and(|i| !i.is_empty()),
            None => self.store.is_dirty(),
        }
    }

    fn select(&mut self, item: Option<&TexItem>) {
        match item {
            None => {
                self.selected.clear();
                self.preview = None;
            }
            Some(item) => {
                self.selected = item.name.clone();
                let path = item.picture.clone();
                let exec = self.editor.exec.clone();
                let pending = Pending::spawn(&self.editor.exec, async move {
                    exec.io(move || {
                        let bytes = std::fs::read(&path).ok()?;
                        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
                        let tile = texture_tile(&bytes, ext.as_deref(), 512).ok()?;
                        Some(egui::ColorImage::from_rgba_unmultiplied(
                            [tile.image.width as usize, tile.image.height as usize],
                            &tile.image.rgba,
                        ))
                    })
                    .await
                });
                self.preview = Some(Preview { name: item.name.clone(), pending, texture: None });
            }
        }
    }

    /// Writes what changed as a job: descriptions, then game textures; New Textures copies its
    /// pictures into the library first, after asking when that replaces a texture the library has.
    /// `by_caller`, a command's caller asked: nothing is asked, an import that replaces is
    /// refused, and what stops the save is that caller's answer.
    fn queue_save(&mut self, by_caller: bool) -> Result<(), String> {
        if self.read_only() {
            return Err("The texture folder cannot be written.".to_owned());
        }
        if self.saving() {
            // a job of the library runs: this save starts when it has ended
            *self.save_error.lock() = None;
            super::save_after_running(&self.editor, &self.save_error, |ed| {
                start_texture_save(ed, None, None)
            });
            return Ok(());
        }
        *self.save_error.lock() = None;
        let Some(items) = self.import.as_ref().and_then(Pending::get).map(|i| i.as_ref().clone()) else {
            let id = start_texture_save(&self.editor, None, None)?;
            super::answer_job(&self.editor, id, &self.save_error);
            return Ok(());
        };
        if by_caller {
            import_for_caller(&self.editor, items, &self.save_error);
        } else {
            confirm_import(&self.editor, items, &self.save_error);
        }
        Ok(())
    }

    /// An import ended: New Textures gives way to the library, which counts what is still new.
    fn after_import(&mut self) {
        if self.import_mode() {
            self.back_to_library();
        } else {
            let (d, exec) = (self.dirs.clone(), self.editor.exec.clone());
            self.new_pictures =
                Pending::spawn(
                    &self.editor.exec,
                    async move { exec.io(move || list_import(&d).len()).await },
                );
        }
    }

    /// Pictures dropped on the tab (from Explorer, or tiles of the Content Browser) are imported.
    fn take_drop(&mut self, ui: &mut Ui, full: Rect) {
        let takes = |p: &gz_ui::drop::Payload| p.files().iter().any(|f| is_picture(f));
        let Some(dropped) = gz_ui::drop::Zone::new("Texture Library", full, &takes).catch_all().take(ui)
        else {
            return;
        };
        let files: Vec<String> =
            dropped.files().iter().filter(|f| is_picture(f)).map(|f| f.display().to_string()).collect();
        if !files.is_empty() {
            let mut a = Args::new();
            a.insert("from".into(), json!(files));
            self.editor.post("library.import", a);
        }
    }

    fn draw_preview(&mut self, ui: &mut Ui, item: &TexItem) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let params = self.params_of(item);
        let alpha = params.flags.contains(TextureFlags::HAS_ALPHA);
        let facts = crate::local::local_facts(
            &ctx,
            &[
                Tx::raw(format!("{} x {}", params.width, params.height)),
                params.fmt.label().map_or(Tx::from("unknown"), format_word),
                if alpha { Tx::from("alpha") } else { Tx::EMPTY },
            ],
        );
        let src = item.picture.to_string_lossy().into_owned();
        local_heading(
            ui,
            Tx::raw(crate::paths::label_of(&item.name)),
            Clip::End,
            &[(Tx::raw(src), Clip::Start), (facts, Clip::End)],
        );
        ui.add_space(8.0);
        let Some(pv) = self.preview.as_mut().filter(|v| v.name == item.name) else { return };
        if pv.texture.is_none()
            && let Some(img) = pv.pending.get()
            && let Some(img) = img.as_ref()
        {
            pv.texture = Some(ctx.load_texture(
                format!("gz-ui-previews.texlib.{}", item.name),
                img.clone(),
                egui::TextureOptions::LINEAR,
            ));
        }
        let Some(tex) = &pv.texture else {
            if pv.pending.get().is_some() {
                text::status(ui, Tone::Warning, "No preview: the texture file was not found.");
            } else {
                state::busy_line(ui, "Reading the picture...");
            }
            return;
        };
        // the picture at the texture's own aspect, as big as the pane allows up to 512 pt, on a checkerboard for alpha
        let avail = ui.available_size();
        let [tw, th] = tex.size().map(|v| v.max(1) as f32);
        let scale = (avail.x / tw).min(avail.y / th).min(IMAGE_MAX / tw.max(th));
        if scale <= 0.0 {
            return;
        }
        let size = Vec2::new(tw * scale, th * scale);
        let (row, _) = ui.allocate_exact_size(Vec2::new(avail.x, size.y), Sense::hover());
        let rect = Rect::from_min_size(pos2(row.min.x + (avail.x - size.x) * 0.5, row.min.y), size);
        if alpha {
            gz_ui::checker(ui.painter(), rect.min, rect, 8.0);
        }
        ui.painter().image(
            tex.id(),
            rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        ui.painter().rect_stroke(
            rect,
            CornerRadius::ZERO,
            egui::Stroke::new(1.0, p.border_subtle),
            egui::StrokeKind::Outside,
        );
    }

    /// The rows a description has, by its type.
    fn draw_properties(&mut self, ui: &mut Ui, item: &TexItem, names: Names<'_>) {
        let before = self.params_of(item);
        let mut p = before.clone();
        let ro = if self.read_only() || self.saving() { PropFlags::READ_ONLY } else { PropFlags::NONE };
        let mut gesture = String::new();
        PropertyGrid::new("texture_library").show(ui, |g| {
            token(
                g,
                "Type".into(),
                "What the game makes of the texture",
                ro,
                &mut p.ty.0,
                &TYPE_TOKENS,
                Tx::from,
                &mut gesture,
            );
            if p.ty != before.ty {
                p.apply_type_defaults();
            }
            g.section("Source", true, None, |g| {
                g.row("Width", "", PropFlags::READ_ONLY, 0, |ui| {
                    read_only(ui, Tx::raw(p.width.to_string()), Clip::End)
                });
                g.row("Height", "", PropFlags::READ_ONLY, 0, |ui| {
                    read_only(ui, Tx::raw(p.height.to_string()), Clip::End)
                });
                let alpha = if p.flags.contains(TextureFlags::HAS_ALPHA) { "present" } else { "absent" };
                g.row("Alpha", "", PropFlags::READ_ONLY, 0, |ui| read_only(ui, alpha, Clip::End));
            });
            match p.ty {
                TextureType::IMAGE | TextureType::CUBE_MAP => {
                    token(
                        g,
                        "Format".into(),
                        "How the game texture is stored",
                        ro,
                        &mut p.fmt.0,
                        &format_tokens(),
                        format_word,
                        &mut gesture,
                    );
                    g.section("Mip Maps", true, None, |g| {
                        flag(g, "Enabled", ro, &mut p.flags, TextureFlags::GENERATE_MIP_MAPS, &mut gesture);
                        token(
                            g,
                            "Filter".into(),
                            "How the smaller levels are made",
                            ro,
                            &mut p.mip_filter.0,
                            &mip_tokens(),
                            Tx::raw,
                            &mut gesture,
                        );
                    });
                    g.section("Bump", true, None, |g| {
                        let was = p.bump_mode;
                        token(g, "Mode".into(), "", ro, &mut p.bump_mode.0, &BUMP_TOKENS, Tx::from, &mut gesture);
                        if p.bump_mode != was {
                            p.apply_type_defaults();
                        }
                        if p.bump_mode == BumpMode::USE || p.bump_mode == BumpMode::USE_PARALLAX {
                            pick(
                                g,
                                ("bump", "Texture"),
                                "The bump map it is lit with",
                                ro,
                                &mut p.bump_name,
                                names,
                                &mut gesture,
                            );
                        }
                    });
                    details(g, ro, &mut p, names, &mut gesture);
                    g.section("Flags", true, None, |g| {
                        flag(g, "Dither", ro, &mut p.flags, TextureFlags::DITHER_COLOR, &mut gesture);
                        flag(
                            g,
                            "Dither Each Mip",
                            ro,
                            &mut p.flags,
                            TextureFlags::DITHER_EACH_MIP_LEVEL,
                            &mut gesture,
                        );
                        flag(
                            g,
                            "Implicitly Lit",
                            ro,
                            &mut p.flags,
                            TextureFlags::IMPLICIT_LIGHTED,
                            &mut gesture,
                        );
                    });
                    g.section("Fade", true, None, |g| {
                        flag(g, "Fade Color", ro, &mut p.flags, TextureFlags::FADE_TO_COLOR, &mut gesture);
                        flag(g, "Fade Alpha", ro, &mut p.flags, TextureFlags::FADE_TO_ALPHA, &mut gesture);
                        let mut d = i32::from(p.fade_delay);
                        whole(g, "First Faded Mip", ro, &mut d, 0, 255, "fade_delay", &mut gesture);
                        p.fade_delay = u8::try_from(d).unwrap_or(0);
                        let mut a = i32::try_from(p.fade_amount).unwrap_or(0);
                        whole(g, "Amount, %", ro, &mut a, 0, 100, "fade_amount", &mut gesture);
                        p.fade_amount = u32::try_from(a).unwrap_or(0);
                        argb(g, "Color", ro, &mut p.fade_color, "fade_color", &mut gesture);
                    });
                    g.section("Border", true, None, |g| {
                        flag(g, "Border Color", ro, &mut p.flags, TextureFlags::COLOR_BORDER, &mut gesture);
                        flag(g, "Border Alpha", ro, &mut p.flags, TextureFlags::ALPHA_BORDER, &mut gesture);
                        argb(g, "Color", ro, &mut p.border_color, "border_color", &mut gesture);
                    });
                }
                TextureType::BUMP_MAP => {
                    g.section("Bump", true, None, |g| {
                        pick(
                            g,
                            ("normal", "Normal Map"),
                            "A normal map of its own, 32 bit, of the same size",
                            ro,
                            &mut p.ext_normal_map_name,
                            names,
                            &mut gesture,
                        );
                        let mut h = p.bump_virtual_height;
                        real(g, "Virtual Height", ro, &mut h, 0.001, "m", "bump_height", &mut gesture);
                        p.bump_virtual_height = h.clamp(0.0, 0.1);
                    });
                }
                TextureType::NORMAL_MAP => {
                    token(
                        g,
                        "Format".into(),
                        "How the game texture is stored",
                        PropFlags::READ_ONLY,
                        &mut p.fmt.0,
                        &format_tokens(),
                        format_word,
                        &mut gesture,
                    );
                    g.section("Mip Maps", true, None, |g| {
                        flag(g, "Enabled", ro, &mut p.flags, TextureFlags::GENERATE_MIP_MAPS, &mut gesture);
                        token(
                            g,
                            "Filter".into(),
                            "How the smaller levels are made",
                            ro,
                            &mut p.mip_filter.0,
                            &mip_tokens(),
                            Tx::raw,
                            &mut gesture,
                        );
                    });
                }
                _ => {
                    token(
                        g,
                        "Format".into(),
                        "How the game texture is stored",
                        PropFlags::READ_ONLY,
                        &mut p.fmt.0,
                        &format_tokens(),
                        format_word,
                        &mut gesture,
                    );
                    details(g, ro, &mut p, names, &mut gesture);
                    g.section("Flags", true, None, |g| {
                        flag(
                            g,
                            "Implicitly Lit",
                            PropFlags::READ_ONLY,
                            &mut p.flags,
                            TextureFlags::IMPLICIT_LIGHTED,
                            &mut gesture,
                        );
                    });
                }
            }
            // the strip made into a smaller one beside its game texture
            if p.ty == TextureType::CUBE_MAP && !self.import_mode() {
                let mut make = false;
                g.section("Cube Map", true, None, |g| {
                    g.row(
                        "Smaller Copy",
                        "Makes <name>#small.dds in the game textures: 32 x 32 a face, sampled from the source.",
                        PropFlags::NONE,
                        0,
                        |ui| {
                            make = Button::new("Make").kind(Kind::Secondary).show(ui).clicked();
                        },
                    );
                });
                if make && let Err(e) = crate::libtools::smaller_cube_map(&self.editor, &item.name) {
                    self.editor.notices.error(e);
                }
            }
        });
        if p != before && ro == PropFlags::NONE {
            let gesture = format!("{}:{gesture}", item.name);
            self.store
                .post(TexCmd::Set { name: item.name.clone(), params: p, gesture }, self.editor.ui_origin());
        }
    }

    fn state(&self, items: &[TexItem]) -> Map<String, Value> {
        let keys: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        let rows = shown(&keys, &|i| items.get(i).is_some_and(|t| self.visible(t)), &self.list.search);
        let mut out = Map::new();
        out.insert("shown".into(), json!(rows.len()));
        out.insert("first".into(), json!(rows.first().and_then(|&i| keys.get(i)).copied().unwrap_or("")));
        out.insert("items".into(), json!(items.len()));
        out.insert("import".into(), json!(self.import_mode()));
        let shown_types: Vec<&str> =
            TYPE_WORDS.iter().zip(self.filter).filter(|(_, on)| *on).map(|(w, _)| *w).collect();
        out.insert("types".into(), json!(shown_types));
        out.insert("read_only".into(), json!(self.read_only()));
        out.insert("saving".into(), json!(self.saving()));
        out.insert("dirty".into(), json!(self.dirty_now()));
        out.insert("selected".into(), json!(self.selected));
        let folder = if self.import_mode() { &self.dirs.import } else { &self.dirs.textures };
        out.insert("folder".into(), json!(crate::paths::json_path(folder)));
        if let Some(item) = items.iter().find(|i| i.name == self.selected) {
            let p = self.params_of(item);
            out.insert("width".into(), json!(p.width));
            out.insert("height".into(), json!(p.height));
            out.insert("alpha".into(), json!(p.flags.contains(TextureFlags::HAS_ALPHA)));
            out.insert("format".into(), json!(p.fmt.label().unwrap_or("")));
            let preview = self
                .preview
                .as_ref()
                .filter(|v| v.name == item.name)
                .and_then(|v| v.pending.get())
                .is_some_and(|i| i.is_some());
            out.insert("preview".into(), json!(preview));
        }
        out
    }

    /// The list a command reads: awaited, then answered with what the tab holds.
    pub fn import_pending(&self) -> Option<Pending<Vec<TexItem>>> {
        self.import.clone()
    }

    /// Applies `search=` and `select` against the list `items` (read by the command).
    pub fn mcp_with(
        &mut self,
        action: &str,
        args: &Args,
        items: &[TexItem],
    ) -> Result<Map<String, Value>, String> {
        if let Some(s) = args.get("search").and_then(Value::as_str) {
            self.list.search = s.to_owned();
        }
        if let Some(types) = args.get("types").and_then(Value::as_array) {
            let mut filter = [false; 5];
            for t in types.iter().filter_map(Value::as_str) {
                let at = TYPE_WORDS
                    .iter()
                    .position(|w| *w == t)
                    .ok_or_else(|| format!("types takes {}", TYPE_WORDS.join(", ")))?;
                filter[at] = true;
            }
            self.filter = filter;
        }
        if action == "check" {
            self.start_import();
            return Ok(Map::new());
        }
        if action == "select" {
            let item = args.get("item").and_then(Value::as_str).unwrap_or_default();
            let keys: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
            let Some(at) = find(&keys, item) else {
                return Err(format!("no texture is named or matches '{item}'"));
            };
            let picked = items.get(at).cloned();
            self.select(picked.as_ref());
        }
        Ok(self.state(items))
    }
}

pub(crate) const TYPE_TOKENS: [(u32, &str); 5] =
    [(0, "2D Texture"), (1, "Cube Map"), (2, "Bump Map"), (3, "Normal Map"), (4, "Terrain")];
pub(crate) const BUMP_TOKENS: [(u32, &str); 3] = [(1, "None"), (2, "Use"), (3, "Use parallax")];

pub(crate) fn format_tokens() -> Vec<(u32, &'static str)> {
    (0..=17).filter_map(|v| TextureFormat(v).label().map(|l| (v, l))).collect()
}

pub(crate) fn mip_tokens() -> Vec<(u32, &'static str)> {
    (0..=14).filter_map(|v| MipFilterId(v).label().map(|l| (v, l))).collect()
}

pub(crate) fn material_tokens() -> Vec<(u32, &'static str)> {
    (0..=3).filter_map(|v| TextureMaterial(v).label().map(|l| (v, l))).collect()
}

/// A format's name in the window: one with words ("DXT1 Alpha", "8 bit (alpha)") is translated, a
/// code ("DXT5", "BC7") is shown as it is.
fn format_word(label: &'static str) -> Tx<'static> {
    if label.contains(' ') { Tx::from(label) } else { Tx::raw(label) }
}

/// A row picking one of a token list; a value the list lacks is shown by its number. `shown`
/// says whether a token's name is a word of the editor's own or a name shown as it is.
#[allow(clippy::too_many_arguments)]
fn token(
    g: &mut Grid<'_>,
    label: Tx<'static>,
    help: &'static str,
    flags: PropFlags,
    value: &mut u32,
    tokens: &[(u32, &'static str)],
    shown: fn(&'static str) -> Tx<'static>,
    gesture: &mut String,
) {
    let key = label.en().into_owned();
    g.row(label, help, flags, 0, |ui| {
        let unknown = format!("{value}");
        let mut names: Vec<Tx<'_>> = tokens.iter().map(|(_, n)| shown(n)).collect();
        let mut cur = tokens.iter().position(|(v, _)| v == value).unwrap_or_else(|| {
            names.push(Tx::raw(unknown.as_str()));
            names.len() - 1
        });
        if flags.contains(PropFlags::READ_ONLY) {
            read_only(ui, names.get(cur).cloned().unwrap_or_default(), Clip::End);
        } else if combo(ui, &key, &mut cur, &names)
            && let Some((v, _)) = tokens.get(cur)
        {
            *value = *v;
            gesture.clone_from(&key);
        }
    });
}

fn flag(
    g: &mut Grid<'_>,
    label: &'static str,
    flags: PropFlags,
    value: &mut TextureFlags,
    bit: TextureFlags,
    gesture: &mut String,
) {
    g.row(label, "", flags | PropFlags::INLINE, 0, |ui| {
        let mut on = value.contains(bit);
        if flags.contains(PropFlags::READ_ONLY) {
            read_only(ui, if on { "Yes" } else { "No" }, Clip::End);
        } else if switch(ui, &mut on).changed() {
            value.set(bit, on);
            *gesture = label.to_owned();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn whole(
    g: &mut Grid<'_>,
    label: &'static str,
    flags: PropFlags,
    value: &mut i32,
    lo: i32,
    hi: i32,
    key: &str,
    gesture: &mut String,
) {
    g.row(label, "", flags, 0, |ui| {
        if flags.contains(PropFlags::READ_ONLY) {
            read_only(ui, Tx::raw(value.to_string()), Clip::End);
            return;
        }
        let before = *value;
        gz_ui::grid::integer(ui, value, 1.0, "");
        *value = (*value).clamp(lo, hi);
        if *value != before {
            *gesture = key.to_owned();
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn real(
    g: &mut Grid<'_>,
    label: &'static str,
    flags: PropFlags,
    value: &mut f32,
    speed: f32,
    unit: &'static str,
    key: &str,
    gesture: &mut String,
) {
    g.row(label, "", flags, 0, |ui| {
        if flags.contains(PropFlags::READ_ONLY) {
            read_only(ui, Tx::raw(format!("{value}")), Clip::End);
            return;
        }
        let before = *value;
        number(ui, value, speed, unit);
        if *value != before {
            *gesture = key.to_owned();
        }
    });
}

/// `0xAARRGGBB` as a swatch and its hex, the alpha kept.
fn argb(
    g: &mut Grid<'_>,
    label: &'static str,
    flags: PropFlags,
    value: &mut u32,
    key: &str,
    gesture: &mut String,
) {
    g.row(label, "", flags, 0, |ui| {
        let v = *value;
        let mut c =
            Color32::from_rgba_unmultiplied((v >> 16) as u8, (v >> 8) as u8, v as u8, (v >> 24) as u8);
        if flags.contains(PropFlags::READ_ONLY) {
            read_only(ui, Tx::raw(gz_ui::grid::color_hex(c)), Clip::End);
        } else if color(ui, &mut c).changed() {
            let [r, gr, b, a] = c.to_srgba_unmultiplied();
            *value = (u32::from(a) << 24) | (u32::from(r) << 16) | (u32::from(gr) << 8) | u32::from(b);
            *gesture = key.to_owned();
        }
    });
}

/// A texture of the library picked from its list, or none.
fn pick(
    g: &mut Grid<'_>,
    (key, label): (&str, &'static str),
    help: &'static str,
    flags: PropFlags,
    value: &mut gz_editor::SmolStr,
    names: Names<'_>,
    gesture: &mut String,
) {
    g.row(label, help, flags, 0, |ui| {
        if flags.contains(PropFlags::READ_ONLY) {
            read_only(
                ui,
                if value.is_empty() { Tx::from("None") } else { Tx::raw(value.as_str()) },
                Clip::Start,
            );
            return;
        }
        if let Some(picked) = texture_pick(ui, key, value, names) {
            *value = picked.into();
            *gesture = key.to_owned();
        }
    });
}

fn details(g: &mut Grid<'_>, ro: PropFlags, p: &mut TextureParams, names: Names<'_>, gesture: &mut String) {
    g.section(Tx::ctx("texture_detail", "Details"), true, None, |g| {
        flag(g, "Use As Diffuse", ro, &mut p.flags, TextureFlags::DIFFUSE_DETAIL, gesture);
        flag(g, "Use As Bump", ro, &mut p.flags, TextureFlags::BUMP_DETAIL, gesture);
        pick(
            g,
            ("detail", "Texture"),
            "The detail texture laid over it",
            ro,
            &mut p.detail_name,
            names,
            gesture,
        );
        let mut s = p.detail_scale;
        real(g, "Scale", ro, &mut s, 0.1, "", "detail_scale", gesture);
        p.detail_scale = s.clamp(0.1, 10000.0);
    });
    g.section("Material", true, None, |g| {
        token(
            g,
            Tx::ctx("texture_material", "Base"),
            "The lighting model the game blends from",
            ro,
            &mut p.material.0,
            &material_tokens(),
            Tx::raw,
            gesture,
        );
        let mut w = p.material_weight;
        real(g, "Weight", ro, &mut w, 0.01, "", "material_weight", gesture);
        p.material_weight = w;
    });
}

/// The picture files the library imports, by extension.
pub const PICTURE_EXTS: [&str; 5] = ["tga", "bmp", "dds", "png", "jpg"];

/// Whether a file is a picture the library imports.
#[must_use]
pub fn is_picture(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|e| PICTURE_EXTS.iter().any(|p| e.eq_ignore_ascii_case(p)))
}

/// The library names an import of `items` would replace: the textures it has already.
#[must_use]
pub fn replaced_by_import(dirs: &EditorDirs, items: &[TexItem]) -> Vec<String> {
    items
        .iter()
        .filter_map(|i| {
            let name = texture_folder_name(crate::paths::strip_ext(&i.name));
            dirs.textures.join(format!("{}.tga", name.replace('/', "\\"))).is_file().then_some(name)
        })
        .collect()
}

/// New Textures' Import for a command's caller: nothing is asked. An import that replaces
/// textures of the library is refused, naming them and the command that replaces them.
fn import_for_caller(editor: &Arc<Editor>, items: Vec<TexItem>, failed: &super::SaveError) {
    let (dirs, ed, failed) = (EditorDirs::of(editor), editor.clone(), failed.clone());
    let _task = editor.exec.clone().spawn(async move {
        let (d, i) = (dirs, items.clone());
        let replaced = ed.exec.io(move || replaced_by_import(&d, &i)).await;
        let refused = if replaced.is_empty() {
            match start_texture_save(&ed, Some(items), None) {
                Ok(id) => {
                    super::answer_job(&ed, id, &failed);
                    return;
                }
                Err(e) => CmdError::new("failed", e),
            }
        } else {
            CmdError::new(
                "exists",
                format!(
                    "The import would replace: {}. library.import overwrite=true replaces them.",
                    replaced.join(", ")
                ),
            )
        };
        *failed.lock() = Some(refused);
        ed.exec.wake().ping();
    });
}

/// New Textures' Import: asks first when it replaces textures of the library, then starts.
fn confirm_import(editor: &Arc<Editor>, items: Vec<TexItem>, failed: &super::SaveError) {
    let (dirs, ed, failed) = (EditorDirs::of(editor), editor.clone(), failed.clone());
    let asker = if editor.flags.unattended { gz_ui::Asker::Unattended } else { gz_ui::Asker::Ui };
    editor.exec.clone().spawn(async move {
        let (d, i) = (dirs, items.clone());
        let replaced = ed.exec.io(move || replaced_by_import(&d, &i)).await;
        let Ok(queue) = gz_app::UiQueue::of(&ed) else { return };
        queue.post(move |_shell, ctx| {
            // every end of the import is the save's answer: its job's, a start that fails, the
            // question answered No
            let answer = {
                let (ed, failed) = (ed.clone(), failed.clone());
                move |why: CmdError| {
                    *failed.lock() = Some(why);
                    ed.exec.wake().ping();
                }
            };
            let start = {
                let (ed, items, failed, answer) = (ed.clone(), items, failed.clone(), answer.clone());
                move || match start_texture_save(&ed, Some(items), None) {
                    Ok(id) => super::answer_job(&ed, id, &failed),
                    Err(e) => answer(CmdError::new("failed", e)),
                }
            };
            if replaced.is_empty() {
                start();
                return;
            }
            let spec = gz_ui::ConfirmSpec {
                id: "import_replaces".to_owned(),
                title: "Import New Textures".into(),
                headline: "These pictures replace textures the library has:".into(),
                items: replaced.iter().take(10).cloned().map(Tx::raw).collect(),
                total: replaced.len(),
                consequence: "Their source pictures are replaced and their game textures built again.".into(),
                action: "Replace".into(),
                destructive: true,
            };
            let line = gz_ui::dialog::confirm(ctx, asker, spec, move |reply| {
                if reply == gz_ui::Answer::Yes {
                    start();
                } else {
                    answer(CmdError::new(gz_app::SAVE_CANCELLED, "the import was cancelled"));
                }
            });
            if let Some(line) = line {
                ed.log.finding(&line);
            }
        });
    });
}

/// The pictures of `items` taken into the library and built, one after another.
///
/// `staged` names the pictures (by file name) the import itself copied into the import folder:
/// they are removed once they are in the library, where the author's own are renamed out of the way.
pub(crate) fn import_items(
    dirs: &LibraryWriter,
    items: &[TexItem],
    params_of: &dyn Fn(&TexItem) -> TextureParams,
    staged: &[String],
    c: &gz_task::JobCtx,
) -> Result<(usize, Vec<String>), String> {
    let age = super::texsync::now();
    let (mut done, mut failed) = (0usize, Vec::new());
    for (i, item) in items.iter().enumerate() {
        c.check().map_err(|_| "Stopped.".to_owned())?;
        c.stage(format!("Importing {}", item.name));
        c.progress(i, items.len());
        let params = params_of(item);
        let mine = staged.iter().any(|n| n.eq_ignore_ascii_case(&item.name));
        match import_picture(dirs, &item.name, mine)
            .and_then(|name| sync_texture(dirs, &name, Some(&params), true, true, age))
        {
            Ok(s) => {
                done += 1;
                if s.low_gloss {
                    failed.push(format!("The gloss mask of '{}' is under 10 percent.", item.name));
                }
            }
            Err(e) => failed.push(e),
        }
    }
    Ok((done, failed))
}

/// Writes the edited descriptions and builds their game textures - or, with `import`, takes
/// those pictures into the library - as one job. `only` limits the edits written to those
/// entries. The window's Save and `library.set` start it alike.
pub fn start_texture_save(
    editor: &Arc<Editor>,
    import: Option<Vec<TexItem>>,
    only: Option<Vec<String>>,
) -> Result<JobId, String> {
    let stores = LibraryStores::of(editor);
    if stores.tex_saving.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return Err("The texture library is saving.".to_owned());
    }
    let edits: std::collections::BTreeMap<String, TextureParams> = stores
        .tex
        .snapshot()
        .edits
        .iter()
        .filter(|(k, _)| only.as_ref().is_none_or(|o| o.contains(k)))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let partial = only.is_some();
    let (dirs, ed, store, saving) =
        (LibraryWriter::of(editor), editor.clone(), stores.tex.clone(), stores.tex_saving.clone());
    let importing = import.is_some();
    let title = if importing { "Importing New Textures" } else { "Saving the Texture Library" };
    let started = editor.jobs.start("texture_library", Some("libraries"), title, move |ctx| async move {
        let library =
            if importing { Vec::new() } else { TexLibDoc::items_now(&ed, None).await.as_ref().clone() };
        let written: Vec<String> = edits.keys().cloned().collect();
        let c = ctx.clone();
        let result = ctx
            .exec
            .io(move || -> Result<Written, String> {
                if let Some(items) = import {
                    let params_of = |item: &TexItem| {
                        edits.get(&item.name).cloned().unwrap_or_else(|| item.params.clone())
                    };
                    return import_items(&dirs, &items, &params_of, &[], &c)
                        .map(|(n, f)| (n, f, Vec::new(), Vec::new()));
                }
                let age = super::texsync::now();
                // `kept`: the textures that could not be written - their edits stay; `notes`: what
                // was written and still deserves a word
                let (mut done, mut failed, mut kept, mut notes) =
                    (0usize, Vec::new(), Vec::new(), Vec::new());
                for (i, (name, params)) in edits.iter().enumerate() {
                    c.check().map_err(|_| "Stopped.".to_owned())?;
                    c.stage(format!("Building {name}"));
                    c.progress(i, edits.len());
                    if !library.iter().any(|t| t.name == *name) {
                        continue;
                    }
                    match sync_texture(&dirs, name, Some(params), true, true, age) {
                        Ok(s) => {
                            done += 1;
                            if s.low_gloss {
                                notes.push(format!("The gloss mask of '{name}' is under 10 percent."));
                            }
                        }
                        Err(e) => {
                            failed.push(e);
                            kept.push(name.clone());
                        }
                    }
                }
                Ok((done, failed, kept, notes))
            })
            .await;
        saving.store(false, std::sync::atomic::Ordering::Release);
        LibraryLists::of(&ed).textures_rev.bump();
        match result {
            Ok((done, failed, kept, notes)) => {
                for f in &failed {
                    ed.log.error(f.clone());
                }
                for n in &notes {
                    ed.log.warn(n.clone());
                }
                // what was written is unsaved no more; what could not be written stays an edit,
                // and `failed` is the save's answer for whoever waits for it - the job raises no
                // notice of its own
                let dropped = if partial || !kept.is_empty() {
                    TexCmd::Drop { names: written.into_iter().filter(|n| !kept.contains(n)).collect() }
                } else {
                    TexCmd::Reset
                };
                let _ = store.send(dropped, Origin::Ui).await;
                if importing {
                    after_import_ui(&ed);
                }
                Ok(json!({ "written": done, "failed": failed, "notes": notes }))
            }
            Err(e) => Err(gz_task::JobError::new(e)),
        }
    });
    started.map_err(|busy| {
        stores.tex_saving.store(false, std::sync::atomic::Ordering::Release);
        format!("The save could not be started: {busy}.")
    })
}

/// The tab, when one is open, hears that an import ended.
pub(crate) fn after_import_ui(editor: &Arc<Editor>) {
    if let Ok(q) = gz_app::UiQueue::of(editor) {
        q.post(|shell, _ctx| {
            if let Some(doc) = shell
                .document_mut(KIND)
                .and_then(|d| d.as_any_mut())
                .and_then(|a| a.downcast_mut::<TexLibDoc>())
            {
                doc.after_import();
            }
        });
    }
}

/// What `library.import` brings in.
pub struct ImportRequest {
    /// Picture files anywhere on the disk; none: what the import folder already holds.
    pub from: Vec<PathBuf>,
    /// Description keys applied to every picture imported (`library.set`'s).
    pub set: Map<String, Value>,
    /// Replace what is there: a staged file of the same name, a texture of the library.
    pub overwrite: bool,
    /// Answer when the job ends; false answers the job at once.
    pub wait: bool,
}

/// What an import would do: the copies into the import folder, the file names it imports, and
/// what it would replace.
struct ImportPlan {
    copies: Vec<(PathBuf, PathBuf)>,
    names: Vec<String>,
    replaces: Vec<String>,
}

fn plan_import(dirs: &EditorDirs, from: &[PathBuf]) -> Result<ImportPlan, String> {
    let mut plan = ImportPlan { copies: Vec::new(), names: Vec::new(), replaces: Vec::new() };
    if from.is_empty() {
        plan.names =
            list_files(&dirs.import, &PICTURE_EXTS, true, false).into_iter().map(|(n, _)| n).collect();
    }
    for f in from {
        if !f.is_file() {
            return Err(format!("{} is no file", f.display()));
        }
        if !is_picture(f) {
            return Err(format!(
                "{} is no picture the library imports ({})",
                f.display(),
                PICTURE_EXTS.join(", ")
            ));
        }
        let name = f.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        let dest = dirs.import.join(&name);
        let same = dest.to_string_lossy().eq_ignore_ascii_case(&f.to_string_lossy());
        if !same {
            if dest.exists() {
                plan.replaces.push(format!("{name} (already in the import folder)"));
            }
            plan.copies.push((f.clone(), dest));
        }
        plan.names.push(name);
    }
    for n in &plan.names {
        let lib = texture_folder_name(crate::paths::strip_ext(n));
        if dirs.textures.join(format!("{}.tga", lib.replace('/', "\\"))).is_file() {
            plan.replaces.push(lib);
        }
    }
    Ok(plan)
}

/// The blocking half of `library.import`: the pictures copied into the import folder, then every
/// picture the request names (or the folder holds) taken into the library and built. Returns the
/// names imported and the sentences of what failed.
fn stage_and_import(
    d: &LibraryWriter,
    plan: &ImportPlan,
    only_listed: bool,
    set: &Map<String, Value>,
    c: &gz_task::JobCtx,
) -> Result<(Vec<String>, Vec<String>), String> {
    c.stage("Copying the pictures");
    d.create_dir_all(&d.import)?;
    let mut staged = Vec::new();
    for (src, dest) in &plan.copies {
        c.check().map_err(|_| "Stopped.".to_owned())?;
        d.copy(src, dest)?;
        staged.push(dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
    }
    let mut failed = Vec::new();
    let mut items = Vec::new();
    for mut item in list_import(d) {
        if only_listed && !plan.names.iter().any(|n| n.eq_ignore_ascii_case(&item.name)) {
            continue;
        }
        match apply_texture(&mut item.params, set) {
            Ok(_) => items.push(item),
            Err(e) => failed.push(format!("{}: {e}", item.name)),
        }
    }
    let names: Vec<String> = items.iter().map(|i| i.name.clone()).collect();
    let (_, more) = import_items(d, &items, &|i| i.params.clone(), &staged, c)?;
    failed.extend(more);
    Ok((names, failed))
}

/// `library.import`: pictures from anywhere (or the import folder as it is) copied into the
/// import folder and taken into the library, as one job a person can read and stop.
pub async fn import_pictures(editor: &Arc<Editor>, req: ImportRequest) -> Result<Value, CmdError> {
    let dirs = LibraryWriter::of(editor);
    let d = dirs.clone();
    if !editor.exec.io(move || can_write(&d.textures)).await {
        return Err(CmdError::new("read_only", "The texture folder cannot be written."));
    }
    if !req.set.is_empty() {
        apply_texture(&mut TextureParams::default(), &req.set).map_err(CmdError::invalid)?;
    }
    let (d, from) = (dirs.clone(), req.from.clone());
    let plan = editor.exec.io(move || plan_import(&d, &from)).await.map_err(CmdError::invalid)?;
    if plan.names.is_empty() {
        return Ok(json!({
            "imported": Vec::<String>::new(),
            "failed": Vec::<String>::new(),
            "note": format!("Nothing in {} is new to the library.", folder_text(editor, &dirs.import)),
        }));
    }
    if !req.overwrite && !plan.replaces.is_empty() {
        return Err(CmdError::new(
            "exists",
            format!(
                "The import would replace: {}. Pass overwrite=true to replace them.",
                plan.replaces.join(", ")
            ),
        )
        .with_data(json!({ "replaces": plan.replaces })));
    }
    let stores = LibraryStores::of(editor);
    if stores.tex_saving.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return Err(CmdError::busy("The texture library is saving: wait for it or stop its job."));
    }
    let (ed, saving, set) = (editor.clone(), stores.tex_saving.clone(), req.set.clone());
    let only_listed = !req.from.is_empty();
    let started = editor.jobs.start(
        "texture_library",
        Some("libraries"),
        "Importing Textures",
        move |ctx| async move {
            let c = ctx.clone();
            let d = dirs.clone();
            let result = ctx
                .exec
                .io(move || -> Result<(Vec<String>, Vec<String>), String> {
                    let staged = stage_and_import(&d, &plan, only_listed, &set, &c);
                    // what the command copied into the import folder does not stay there,
                    // whether the import went through or not
                    for (_, dest) in &plan.copies {
                        let _ = d.remove_file(dest);
                    }
                    staged
                })
                .await;
            saving.store(false, std::sync::atomic::Ordering::Release);
            LibraryLists::of(&ed).textures_rev.bump();
            after_import_ui(&ed);
            match result {
                Ok((names, failed)) => {
                    for f in &failed {
                        ed.log.error(f.clone());
                    }
                    Ok(json!({ "imported": names, "failed": failed }))
                }
                Err(e) => Err(gz_task::JobError::new(e)),
            }
        },
    );
    let id = started.map_err(|busy| {
        stores.tex_saving.store(false, std::sync::atomic::Ordering::Release);
        CmdError::busy(format!("{busy}"))
    })?;
    if !req.wait {
        return Ok(json!({ "job": id.0 }));
    }
    job_answer(editor, id).await
}

/// What a save wrote: how many, the sentences of what failed, the names whose edits stay (they could
/// not be written), and the notes about what was written.
type Written = (usize, Vec<String>, Vec<String>, Vec<String>);

/// What a job of this library answered, as a command's answer: the job's own result, or why it
/// failed.
pub(crate) async fn job_answer(editor: &Arc<Editor>, id: JobId) -> Result<Value, CmdError> {
    let info = editor.jobs.wait(id).await.ok_or_else(|| CmdError::internal("the job is gone"))?;
    match info.state {
        gz_editor::JobState::Done => {
            let mut v = info.result.map_or(json!({}), |r| (*r).clone());
            v["job"] = json!(id.0);
            if let Some(failed) = v.get("failed").and_then(Value::as_array).filter(|f| !f.is_empty()) {
                let text = failed.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ");
                return Err(CmdError::new("failed", text).with_data(v));
            }
            Ok(v)
        }
        gz_editor::JobState::Cancelled => {
            Err(CmdError::new("failed", "Stopped.").with_data(json!({ "job": id.0, "cancelled": true })))
        }
        _ => Err(CmdError::new("failed", info.error.unwrap_or_else(|| "The job failed.".into()))
            .with_data(json!({ "job": id.0 }))),
    }
}

/// Makes the game texture of `name` again from its source picture and description (the pending
/// edit when `params` is given), as a job.
pub fn start_texture_rebuild(
    editor: &Arc<Editor>,
    name: &str,
    params: Option<TextureParams>,
) -> Result<JobId, String> {
    let stores = LibraryStores::of(editor);
    if stores.tex_saving.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return Err("The texture library is saving.".to_owned());
    }
    let (dirs, ed, saving, name) =
        (LibraryWriter::of(editor), editor.clone(), stores.tex_saving.clone(), name.to_owned());
    let started = editor.jobs.start(
        "texture_library",
        Some("libraries"),
        "Rebuilding the Game Texture",
        move |ctx| async move {
            ctx.stage(format!("Building {name}"));
            let n = name.clone();
            let built = ctx
                .exec
                .io(move || sync_texture(&dirs, &n, params.as_ref(), false, true, super::texsync::now()))
                .await;
            saving.store(false, std::sync::atomic::Ordering::Release);
            ed.exec.wake().ping();
            match built {
                Ok(s) if s.low_gloss => {
                    Ok(json!({ "rebuilt": name, "failed": [format!("The gloss mask of '{name}' is under 10 percent.")] }))
                }
                Ok(_) => Ok(json!({ "rebuilt": name })),
                Err(e) => Err(gz_task::JobError::new(e)),
            }
        },
    );
    started.map_err(|busy| {
        stores.tex_saving.store(false, std::sync::atomic::Ordering::Release);
        format!("The rebuild could not be started: {busy}.")
    })
}

impl LibraryDoc for TexLibDoc {
    fn mcp(&mut self, action: &str, args: &Args) -> Result<Map<String, Value>, String> {
        let items = self.items().unwrap_or_default();
        self.mcp_with(action, args, &items)
    }

    fn settle(&self) -> Option<Settle> {
        let p = self.preview.as_ref()?.pending.clone();
        Some(Box::pin(async move {
            let _ = p.wait(std::time::Duration::from_secs(10)).await;
        }))
    }
}

impl Document for TexLibDoc {
    fn id(&self) -> &str {
        KIND
    }
    fn kind(&self) -> &'static str {
        KIND
    }
    fn label(&self) -> Tx<'static> {
        Tx::from(if self.import_mode() { "New Textures" } else { "Texture Library" })
    }
    fn icon(&self) -> Icon {
        Icon::FileTexture
    }
    fn path(&self) -> Option<String> {
        // New Textures is another target of the same tab: switching to it asks while the library is dirty
        self.import_mode().then(|| "import".to_owned())
    }
    fn tooltip(&self) -> Option<Tx<'static>> {
        Some(Tx::from(if self.import_mode() {
            "Textures found in the import folder"
        } else {
            "The texture descriptions of the editor's library"
        }))
    }
    fn dirty(&self) -> bool {
        self.dirty_now()
    }
    /// The pending edits are the editor's (`LibraryStores` reports, saves and drops them); the
    /// pictures New Textures holds are the tab's alone.
    fn dirty_elsewhere(&self) -> bool {
        !self.import_mode()
    }
    fn padded(&self) -> bool {
        false
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        let dirty = self.dirty_now();
        let import = self.import_mode();
        LibraryFacts::of(&self.editor).set_tex(TexFacts {
            read_only: self.read_only(),
            saving: self.saving(),
            dirty,
            import,
            types: self.filter,
        });
        let on = !self.read_only() && !self.saving() && dirty;
        let tip = if import {
            "Import the new textures into the library and build their game textures"
        } else {
            "Save the changed descriptions and rebuild their game textures"
        };
        let why = if self.read_only() {
            "The texture folder cannot be written"
        } else if self.saving() {
            "The library is saving"
        } else if import {
            "There is nothing to import"
        } else {
            "No unsaved changes"
        };
        tb.group(1);
        // Save and Import are one action: the document's save
        if tb.action(
            "save",
            if import { Icon::Plus } else { Icon::Save },
            if import { "Import" } else { "Save" },
            if on { tip } else { why },
            on,
            false,
            Kind::Ghost,
        ) {
            cx.requests.action("file.save");
        }
        if !import {
            tb.group(0);
            tb.menu(
                "types",
                Icon::Filter,
                "Types",
                "The kinds of texture the list shows",
                Some("texture_library_types"),
                true,
                |ui| {
                    for id in crate::TYPE_ACTIONS {
                        gz_app::menu_row(ui, cx, id, None);
                    }
                },
            );
        }
        tb.group(2);
        if self.read_only() {
            tb.status("Read-only", Tone::Warning);
        } else if self.saving() {
            tb.status("Saving...", Tone::Info);
        }
        tb.group(0);
        tb.menu(
            "more",
            Icon::More,
            "More",
            "Import, synchronize and rewrite by hand",
            Some("texture_library_more"),
            true,
            |ui| {
                gz_app::menu_row(ui, cx, "library.check_textures", None);
                gz_app::menu_row(ui, cx, "library.sync_textures", None);
                gz_app::menu_row(ui, cx, "library.rewrite_descriptions", None);
                ui.separator();
                gz_app::menu_row(ui, cx, "library.reload_textures", None);
            },
        );
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        let full = ui.max_rect();

        // New Textures that found nothing gives way to the library, saying so
        if let Some(found) = self.import.as_ref().and_then(Pending::get)
            && found.is_empty()
        {
            let text_ = tr!(
                "Nothing in {place} is new to the library.",
                place = folder_tx(&self.editor, &self.dirs.import)
            );
            // a line over the tab, never a dialog: a command's caller starts the check too
            self.editor.log.info(text_.en());
            state::toast(ui.ctx(), Tone::Info, text_);
            self.back_to_library();
        }
        let items = self.items();
        let all = items.clone().unwrap_or_default();
        let keys: Vec<&str> = all.iter().map(|i| i.name.as_str()).collect();
        // the one on show leaves with its kind
        if let Some(sel) = all.iter().find(|i| i.name == self.selected)
            && !self.visible(sel)
        {
            self.select(None);
        }
        let rows = shown(&keys, &|i| all.get(i).is_some_and(|t| self.visible(t)), &self.list.search);
        let none = all.is_empty();
        let import = self.import_mode();
        let where_ = if import {
            tr!(
                "Nothing in {place} is new to the library.",
                place = folder_tx(&self.editor, &self.dirs.import)
            )
        } else {
            tr!(
                "The library lists the .tga images in {place}. Drop pictures here to add them.",
                place = folder_tx(&self.editor, &self.dirs.textures)
            )
        };
        // a library with nothing in it says so once, over the whole tab, with what fills it
        if items.is_some() && none && !import {
            let act = state::empty_state(
                ui,
                Icon::FileTexture,
                "No textures",
                &where_,
                (!self.read_only()).then_some(Tx::from("Check New Textures")),
            );
            if act {
                self.start_import();
            }
            self.take_drop(ui, full);
            return;
        }
        let [list_r, preview_r, details_r] = panes(ui, KIND);
        let empty = if items.is_none() {
            Empty { title: Tx::from("Reading the library..."), text: Tx::EMPTY, action: None }
        } else if import {
            Empty { title: Tx::from("No new textures"), text: where_.borrow(), action: None }
        } else {
            Empty {
                title: Tx::from("No textures of these kinds"),
                text: Tx::from("The Types filter hides every texture in the library."),
                action: None,
            }
        };
        let selected = self.selected.clone();
        // what the import folder holds that the library does not: offered over the list
        let fresh = if import { 0 } else { self.new_pictures.get().map_or(0, |n| *n) };
        let mut import_now = false;
        let out = pane(ui, list_r, "texture_library.list.pane", false, false, |ui| {
            if fresh > 0 && !self.read_only() {
                let line = tr!("{n} new {n:picture|pictures} in the import folder.", n = fresh);
                import_now =
                    state::banner(ui, Tone::Info, line, Some(Tx::ctx("texture_library", "Check")), None)
                        .is_some();
                ui.add_space(4.0);
            }
            self.list.draw(ui, KIND, &keys, &rows, &selected, &empty)
        });
        if out.acted || import_now {
            // New Textures: the import folder in this tab
            self.start_import();
        }
        if let Some(i) = out.picked {
            let item = all.get(i).cloned();
            self.select(item.as_ref());
        }
        let current = all.iter().find(|i| i.name == self.selected).cloned();
        pane(ui, preview_r, "texture_library.preview", true, false, |ui| match &current {
            Some(item) => self.draw_preview(ui, item),
            None => {
                state::empty_state(
                    ui,
                    Icon::FileTexture,
                    "No texture selected",
                    "Pick a texture in the list to see its description.",
                    None,
                );
            }
        });
        let names = if import { Names::none() } else { Names::of(&all) };
        // nothing picked: the preview says so, the details stay blank
        pane(ui, details_r, "texture_library.details", false, true, |ui| {
            if let Some(item) = &current {
                self.draw_properties(ui, item, names);
            }
        });
        ui.allocate_rect(ui.max_rect(), Sense::hover());
        self.take_drop(ui, full);
    }

    fn status(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        let items = self.items().unwrap_or_default();
        let keys: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        let rows = shown(&keys, &|i| items.get(i).is_some_and(|t| self.visible(t)), &self.list.search);
        let changed = self.store.snapshot().edits.len();
        let line = crate::local::local_facts(
            ui.ctx(),
            &[
                tr!("{n} {n:texture|textures}", n = items.len()),
                if rows.len() == items.len() {
                    Tx::EMPTY
                } else {
                    tr!("previews" => "{n} shown", n = rows.len())
                },
                if changed > 0 && !self.import_mode() {
                    tr!("previews" => "{n} changed", n = changed)
                } else {
                    Tx::EMPTY
                },
            ],
        );
        let p = palette(ui.ctx());
        text::label(ui, line, Font::Small, p.text_secondary, Clip::End);
    }

    fn edit(&mut self, _cx: &mut DocCx<'_>, action: EditAction, run: bool) -> bool {
        let h = self.store.history();
        let there = match action {
            EditAction::Undo => !h.undo.is_empty(),
            EditAction::Redo => !h.redo.is_empty(),
            _ => return false,
        };
        if run && there {
            let store = self.store.clone();
            self.editor.exec.spawn(async move {
                match action {
                    EditAction::Undo => store.undo().await,
                    _ => store.redo().await,
                };
            });
        }
        there
    }

    fn save(&mut self, cx: &mut DocCx<'_>) -> Result<(), String> {
        self.queue_save(cx.requests.by_caller())
    }

    fn save_failed(&mut self) -> Option<CmdError> {
        self.save_error.lock().take()
    }

    fn discard(&mut self, _cx: &mut DocCx<'_>) {
        // New Textures: nothing is imported; the library: every description is read again
        if self.import_mode() {
            self.import = None;
        }
        self.store.post(TexCmd::Reset, Origin::Ui);
        LibraryLists::of(&self.editor).textures_rev.bump();
    }

    fn mcp_state(&self) -> Value {
        Value::Object(self.state(&self.items().unwrap_or_default()))
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// The `texture_library` factory: `arg` `import` opens New Textures.
pub fn factory(editor: &Arc<Editor>, req: &OpenRequest) -> Result<Box<dyn Document>, String> {
    let import = req.arg.as_deref() == Some("import") || req.path.as_deref() == Some("import");
    Ok(Box::new(TexLibDoc::new(editor, import)))
}
