//! The Texture Library: the `texture_library` tab over the descriptions of `rawdata\textures`
//! (`UIImageEditorForm.cpp`), and New Textures - the same tab over the import folder - that
//! Check New Textures turns it into. Save writes the changed descriptions and builds their game
//! textures as a job beside the editor.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Color32, CornerRadius, Rect, Sense, TextureHandle, Ui, Vec2, pos2};
use gz_app::{DocCx, Document, EditAction, OpenRequest};
use gz_editor::{Args, ArgsExt, CmdError, Editor, Origin};
use gz_fmt_tex::{
    BumpMode, MipFilterId, TextureFlags, TextureFormat, TextureMaterial, TextureParams, TextureType,
    read_texture_thm, texture_tile,
};
use gz_store::{Apply, State, Store, StoreOptions, imbl};
use gz_ui::grid::{Grid, color, combo, number, read_only, switch};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Kind, MenuItem, PropFlags, PropertyGrid, Tone, Toolbar, menu_item, palette, state};
use serde_json::{Map, Value, json};

use super::list::{Empty, LibraryList, find, shown};
use super::texsync::{import_picture, read_picture, sync_texture};
use super::{LibraryDoc, LibraryLists, Settle, can_write, folder_text, pane, panes};
use crate::local::{local_checker, local_heading};
use crate::paths::{EditorDirs, list_files, texture_folder_name};
use crate::pending::Pending;

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

/// New Textures: the pictures at the import folder's root (`GetLocalNewTextures`), each with a
/// description of its own, the one a game texture of its name has, or a new picture's defaults.
#[must_use]
pub fn list_import(dirs: &EditorDirs) -> Vec<TexItem> {
    let files = list_files(&dirs.import, &["tga", "bmp", "dds", "png", "jpg"], true, false);
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
    read_only: bool,
    /// A save is running.
    saving: Arc<std::sync::atomic::AtomicBool>,
    preview: Option<Preview>,
}

const TYPE_NAMES: [&str; 5] = ["Images", "Cube Maps", "Bump Maps", "Normal Maps", "Terrain"];

impl TexLibDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>, import: bool) -> Self {
        let dirs = EditorDirs::of(editor);
        let read_only = !can_write(&dirs.textures);
        if read_only {
            editor.log.warn("The texture folder cannot be written: texture descriptions are read-only.");
        }
        let mut doc = Self {
            editor: editor.clone(),
            store: Store::spawn(
                "texture library",
                TexEdits::default(),
                &editor.exec,
                StoreOptions::default(),
            ),
            dirs,
            import: None,
            list: LibraryList::default(),
            selected: String::new(),
            filter: [true; 5],
            read_only,
            saving: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
        self.selected.clear();
        self.preview = None;
        self.list.search.clear();
        self.store.post(TexCmd::Reset, Origin::Ui);
        LibraryLists::of(&self.editor).textures_rev.bump();
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

    /// Writes what changed (`UpdateLib`) as a job: descriptions, then game textures; New
    /// Textures copies its pictures into the library first.
    fn queue_save(&mut self) -> Result<(), String> {
        if self.read_only {
            return Err("The texture folder cannot be written.".to_owned());
        }
        if self.saving() {
            return Ok(());
        }
        let edits = self.store.snapshot().edits.clone();
        let import: Option<Vec<TexItem>> =
            self.import.as_ref().and_then(Pending::get).map(|i| i.as_ref().clone());
        let library = self.items().map(|i| i.as_ref().clone()).unwrap_or_default();
        let (dirs, saving, store) = (self.dirs.clone(), self.saving.clone(), self.store.clone());
        let editor = self.editor.clone();
        let importing = import.is_some();
        saving.store(true, std::sync::atomic::Ordering::Release);
        let title = if importing { "Importing New Textures" } else { "Saving the Texture Library" };
        let started =
            self.editor.jobs.start("texture_library", Some("libraries"), title, move |ctx| async move {
                let exec = ctx.exec.clone();
                let c = ctx.clone();
                let result = exec
                    .io(move || -> Result<(usize, Vec<String>), String> {
                        let age = super::texsync::now();
                        let mut failed = Vec::new();
                        let mut done = 0usize;
                        match import {
                            Some(items) => {
                                for (i, item) in items.iter().enumerate() {
                                    c.check().map_err(|_| "Stopped.".to_owned())?;
                                    c.stage(format!("Importing {}", item.name));
                                    c.progress(i, items.len());
                                    let params =
                                        edits.get(&item.name).cloned().unwrap_or_else(|| item.params.clone());
                                    match import_picture(&dirs, &item.name).and_then(|name| {
                                        sync_texture(&dirs, &name, Some(&params), true, true, age)
                                    }) {
                                        Ok(s) => {
                                            done += 1;
                                            if s.low_gloss {
                                                failed.push(format!("Invalid gloss mask '{}'.", item.name));
                                            }
                                        }
                                        Err(e) => failed.push(e),
                                    }
                                }
                            }
                            None => {
                                let total = edits.len();
                                for (i, (name, params)) in edits.iter().enumerate() {
                                    c.check().map_err(|_| "Stopped.".to_owned())?;
                                    c.stage(format!("Building {name}"));
                                    c.progress(i, total);
                                    if !library.iter().any(|t| t.name == *name) {
                                        continue;
                                    }
                                    match sync_texture(&dirs, name, Some(params), true, true, age) {
                                        Ok(s) => {
                                            done += 1;
                                            if s.low_gloss {
                                                failed.push(format!("Invalid gloss mask '{name}'."));
                                            }
                                        }
                                        Err(e) => failed.push(e),
                                    }
                                }
                            }
                        }
                        Ok((done, failed))
                    })
                    .await;
                saving.store(false, std::sync::atomic::Ordering::Release);
                LibraryLists::of(&editor).textures_rev.bump();
                match result {
                    Ok((done, failed)) => {
                        for f in &failed {
                            editor.log.error(f.clone());
                        }
                        let _ = store.send(TexCmd::Reset, Origin::Ui).await;
                        if importing {
                            // the new textures are in the library now: the library they stood in for comes back
                            if let Ok(q) = gz_app::UiQueue::of(&editor) {
                                q.post(|shell, _ctx| {
                                    if let Some(doc) = shell
                                        .document_mut(KIND)
                                        .and_then(|d| d.as_any_mut())
                                        .and_then(|a| a.downcast_mut::<TexLibDoc>())
                                    {
                                        doc.back_to_library();
                                    }
                                });
                            }
                        }
                        if !failed.is_empty() {
                            editor.notices.error(format!(
                                "{} of the textures could not be built: {}",
                                failed.len(),
                                failed.join(" ")
                            ));
                        }
                        Ok(json!({ "written": done, "failed": failed }))
                    }
                    Err(e) => Err(gz_task::JobError::new(e)),
                }
            });
        if let Err(busy) = started {
            self.saving.store(false, std::sync::atomic::Ordering::Release);
            return Err(format!("The save could not be started: {busy}."));
        }
        Ok(())
    }

    fn draw_preview(&mut self, ui: &mut Ui, item: &TexItem) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let params = self.params_of(item);
        let alpha = params.flags.contains(TextureFlags::HAS_ALPHA);
        let fmt = params.fmt.label().unwrap_or("unknown");
        let facts = format!(
            "{} x {} \u{b7} {fmt}{}",
            params.width,
            params.height,
            if alpha { " \u{b7} alpha" } else { "" }
        );
        let src = item.picture.to_string_lossy().into_owned();
        local_heading(
            ui,
            &crate::paths::label_of(&item.name),
            Clip::End,
            &[(&src, Clip::Start), (&facts, Clip::End)],
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
            local_checker(ui.painter(), &p, rect.min, rect);
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

    /// `STextureParams::FillProp`: the rows a description has, by its type.
    fn draw_properties(&mut self, ui: &mut Ui, item: &TexItem, names: &[&str]) {
        let before = self.params_of(item);
        let mut p = before.clone();
        let ro = if self.read_only || self.saving() { PropFlags::READ_ONLY } else { PropFlags::NONE };
        let mut gesture = String::new();
        PropertyGrid::new("texture_library").show(ui, |g| {
            token(
                g,
                "Type",
                "What the game makes of the texture",
                ro,
                &mut p.ty.0,
                &TYPE_TOKENS,
                &mut gesture,
            );
            if p.ty != before.ty {
                p.apply_type_defaults();
            }
            g.section("Source", true, None, |g| {
                g.row("Width", "", PropFlags::READ_ONLY, 0, |ui| {
                    read_only(ui, &p.width.to_string(), Clip::End)
                });
                g.row("Height", "", PropFlags::READ_ONLY, 0, |ui| {
                    read_only(ui, &p.height.to_string(), Clip::End)
                });
                let alpha = if p.flags.contains(TextureFlags::HAS_ALPHA) { "present" } else { "absent" };
                g.row("Alpha", "", PropFlags::READ_ONLY, 0, |ui| read_only(ui, alpha, Clip::End));
            });
            match p.ty {
                TextureType::IMAGE | TextureType::CUBE_MAP => {
                    token(
                        g,
                        "Format",
                        "How the game texture is stored",
                        ro,
                        &mut p.fmt.0,
                        &format_tokens(),
                        &mut gesture,
                    );
                    g.section("MipMaps", true, None, |g| {
                        flag(g, "Enabled", ro, &mut p.flags, TextureFlags::GENERATE_MIP_MAPS, &mut gesture);
                        token(
                            g,
                            "Filter",
                            "How the smaller levels are made",
                            ro,
                            &mut p.mip_filter.0,
                            &mip_tokens(),
                            &mut gesture,
                        );
                    });
                    g.section("Bump", true, None, |g| {
                        let was = p.bump_mode;
                        token(g, "Mode", "", ro, &mut p.bump_mode.0, &BUMP_TOKENS, &mut gesture);
                        if p.bump_mode != was {
                            p.apply_type_defaults();
                        }
                        if p.bump_mode == BumpMode::USE || p.bump_mode == BumpMode::USE_PARALLAX {
                            pick(
                                g,
                                "Texture",
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
                            "Dither Each MIP",
                            ro,
                            &mut p.flags,
                            TextureFlags::DITHER_EACH_MIP_LEVEL,
                            &mut gesture,
                        );
                        flag(
                            g,
                            "Implicit Lighted",
                            ro,
                            &mut p.flags,
                            TextureFlags::IMPLICIT_LIGHTED,
                            &mut gesture,
                        );
                    });
                    g.section("Fade", true, None, |g| {
                        flag(g, "Enable Color", ro, &mut p.flags, TextureFlags::FADE_TO_COLOR, &mut gesture);
                        flag(g, "Enabled Alpha", ro, &mut p.flags, TextureFlags::FADE_TO_ALPHA, &mut gesture);
                        let mut d = i32::from(p.fade_delay);
                        whole(g, "Delay 'n' MIP", ro, &mut d, 0, 255, "fade_delay", &mut gesture);
                        p.fade_delay = u8::try_from(d).unwrap_or(0);
                        let mut a = i32::try_from(p.fade_amount).unwrap_or(0);
                        whole(g, "% of color to fade in", ro, &mut a, 0, 100, "fade_amount", &mut gesture);
                        p.fade_amount = u32::try_from(a).unwrap_or(0);
                        argb(g, "Color", ro, &mut p.fade_color, "fade_color", &mut gesture);
                    });
                    g.section("Border", true, None, |g| {
                        flag(g, "Enabled Color", ro, &mut p.flags, TextureFlags::COLOR_BORDER, &mut gesture);
                        flag(g, "Enabled Alpha", ro, &mut p.flags, TextureFlags::ALPHA_BORDER, &mut gesture);
                        argb(g, "Color", ro, &mut p.border_color, "border_color", &mut gesture);
                    });
                }
                TextureType::BUMP_MAP => {
                    g.section("Bump", true, None, |g| {
                        pick(
                            g,
                            "Special NormalMap",
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
                        "Format",
                        "How the game texture is stored",
                        PropFlags::READ_ONLY,
                        &mut p.fmt.0,
                        &format_tokens(),
                        &mut gesture,
                    );
                    g.section("MipMaps", true, None, |g| {
                        flag(g, "Enabled", ro, &mut p.flags, TextureFlags::GENERATE_MIP_MAPS, &mut gesture);
                        token(
                            g,
                            "Filter",
                            "How the smaller levels are made",
                            ro,
                            &mut p.mip_filter.0,
                            &mip_tokens(),
                            &mut gesture,
                        );
                    });
                }
                _ => {
                    token(
                        g,
                        "Format",
                        "How the game texture is stored",
                        PropFlags::READ_ONLY,
                        &mut p.fmt.0,
                        &format_tokens(),
                        &mut gesture,
                    );
                    details(g, ro, &mut p, names, &mut gesture);
                    g.section("Flags", true, None, |g| {
                        flag(
                            g,
                            "Implicit Lighted",
                            PropFlags::READ_ONLY,
                            &mut p.flags,
                            TextureFlags::IMPLICIT_LIGHTED,
                            &mut gesture,
                        );
                    });
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
        out.insert("read_only".into(), json!(self.read_only));
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
        if let Some(s) = args.str("search") {
            self.list.search = s.to_owned();
        }
        if action == "select" {
            let item = args.str("item").unwrap_or_default();
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

const TYPE_TOKENS: [(u32, &str); 5] =
    [(0, "2D Texture"), (1, "Cube Map"), (2, "Bump Map"), (3, "Normal Map"), (4, "Terrain")];
const BUMP_TOKENS: [(u32, &str); 3] = [(1, "None"), (2, "Use"), (3, "Use parallax")];

fn format_tokens() -> Vec<(u32, &'static str)> {
    (0..=17).filter_map(|v| TextureFormat(v).label().map(|l| (v, l))).collect()
}

fn mip_tokens() -> Vec<(u32, &'static str)> {
    (0..=14).filter_map(|v| MipFilterId(v).label().map(|l| (v, l))).collect()
}

fn material_tokens() -> Vec<(u32, &'static str)> {
    (0..=3).filter_map(|v| TextureMaterial(v).label().map(|l| (v, l))).collect()
}

/// A row picking one of a token list; a value the list lacks is shown by its number.
fn token(
    g: &mut Grid<'_>,
    label: &str,
    help: &str,
    flags: PropFlags,
    value: &mut u32,
    tokens: &[(u32, &str)],
    gesture: &mut String,
) {
    g.row(label, help, flags, 0, |ui| {
        let unknown = format!("{value}");
        let mut names: Vec<&str> = tokens.iter().map(|(_, n)| *n).collect();
        let mut cur = tokens.iter().position(|(v, _)| v == value).unwrap_or_else(|| {
            names.push(&unknown);
            names.len() - 1
        });
        if flags.contains(PropFlags::READ_ONLY) {
            read_only(ui, names.get(cur).copied().unwrap_or(""), Clip::End);
        } else if combo(ui, label, &mut cur, &names)
            && let Some((v, _)) = tokens.get(cur)
        {
            *value = *v;
            *gesture = label.to_owned();
        }
    });
}

fn flag(
    g: &mut Grid<'_>,
    label: &str,
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
    label: &str,
    flags: PropFlags,
    value: &mut i32,
    lo: i32,
    hi: i32,
    key: &str,
    gesture: &mut String,
) {
    g.row(label, "", flags, 0, |ui| {
        if flags.contains(PropFlags::READ_ONLY) {
            read_only(ui, &value.to_string(), Clip::End);
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
    label: &str,
    flags: PropFlags,
    value: &mut f32,
    speed: f32,
    unit: &str,
    key: &str,
    gesture: &mut String,
) {
    g.row(label, "", flags, 0, |ui| {
        if flags.contains(PropFlags::READ_ONLY) {
            read_only(ui, &format!("{value}"), Clip::End);
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
fn argb(g: &mut Grid<'_>, label: &str, flags: PropFlags, value: &mut u32, key: &str, gesture: &mut String) {
    g.row(label, "", flags, 0, |ui| {
        let v = *value;
        let mut c =
            Color32::from_rgba_unmultiplied((v >> 16) as u8, (v >> 8) as u8, v as u8, (v >> 24) as u8);
        if flags.contains(PropFlags::READ_ONLY) {
            read_only(ui, &gz_ui::grid::color_hex(c), Clip::End);
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
    label: &str,
    help: &str,
    flags: PropFlags,
    value: &mut gz_editor::SmolStr,
    names: &[&str],
    gesture: &mut String,
) {
    g.row(label, help, flags, 0, |ui| {
        if flags.contains(PropFlags::READ_ONLY) {
            read_only(ui, if value.is_empty() { "None" } else { value }, Clip::Start);
            return;
        }
        if let Some(picked) = crate::local::local_texture_pick(ui, label, value, names) {
            *value = picked.into();
            *gesture = label.to_owned();
        }
    });
}

fn details(g: &mut Grid<'_>, ro: PropFlags, p: &mut TextureParams, names: &[&str], gesture: &mut String) {
    g.section("Details", true, None, |g| {
        flag(g, "Use As Diffuse", ro, &mut p.flags, TextureFlags::DIFFUSE_DETAIL, gesture);
        flag(g, "Use As Bump (R2)", ro, &mut p.flags, TextureFlags::BUMP_DETAIL, gesture);
        pick(g, "Texture", "The detail texture laid over it", ro, &mut p.detail_name, names, gesture);
        let mut s = p.detail_scale;
        real(g, "Scale", ro, &mut s, 0.1, "", "detail_scale", gesture);
        p.detail_scale = s.clamp(0.1, 10000.0);
    });
    g.section("Material", true, None, |g| {
        token(
            g,
            "Base",
            "The lighting model the game blends from",
            ro,
            &mut p.material.0,
            &material_tokens(),
            gesture,
        );
        let mut w = p.material_weight;
        real(g, "Weight", ro, &mut w, 0.01, "", "material_weight", gesture);
        p.material_weight = w;
    });
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
    fn label(&self) -> String {
        if self.import_mode() { "New Textures".to_owned() } else { "Texture Library".to_owned() }
    }
    fn icon(&self) -> Icon {
        Icon::FileTexture
    }
    fn path(&self) -> Option<String> {
        // New Textures is another target of the same tab: switching to it asks while the library is dirty
        self.import_mode().then(|| "import".to_owned())
    }
    fn tooltip(&self) -> Option<String> {
        Some(if self.import_mode() {
            "Textures found in the import folder".to_owned()
        } else {
            "The texture descriptions of the editor's library".to_owned()
        })
    }
    fn dirty(&self) -> bool {
        self.dirty_now()
    }
    fn padded(&self) -> bool {
        false
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        let dirty = self.dirty_now();
        let import = self.import_mode();
        let on = !self.read_only && !self.saving() && dirty;
        let tip = if import {
            "Import the new textures into the library and build their game textures"
        } else {
            "Save the changed descriptions and rebuild their game textures"
        };
        let why = if self.read_only {
            "The texture folder cannot be written"
        } else if self.saving() {
            "The library is saving"
        } else if import {
            "There is nothing to import"
        } else {
            "No unsaved changes"
        };
        tb.group(1);
        if tb.action(
            "save",
            if import { Icon::Plus } else { Icon::Save },
            if import { "Import" } else { "Save" },
            if on { tip } else { why },
            on,
            false,
            Kind::Ghost,
        ) && let Err(e) = self.queue_save()
        {
            let asker =
                if self.editor.flags.unattended { gz_ui::Asker::Unattended } else { gz_ui::Asker::Ui };
            if let Some(line) = gz_ui::dialog::notice(cx.ctx, asker, Tone::Error, "Save failed", &e) {
                self.editor.log.plain(line);
            }
        }
        if !import {
            tb.group(0);
            let filter = &mut self.filter;
            tb.menu(
                "types",
                Icon::Filter,
                "Types",
                "The kinds of texture the list shows",
                Some("texture_library_types"),
                true,
                |ui| {
                    for (i, name) in TYPE_NAMES.iter().enumerate() {
                        if let Some(f) = filter.get_mut(i)
                            && menu_item(ui, name, MenuItem { selected: *f, ..MenuItem::default() })
                        {
                            *f = !*f;
                        }
                    }
                },
            );
        }
        tb.group(2);
        if self.read_only {
            tb.status("Read-only", Tone::Warning);
        } else if self.saving() {
            tb.status("Saving...", Tone::Info);
        }
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        // New Textures that found nothing gives way to the library, saying so
        if let Some(found) = self.import.as_ref().and_then(Pending::get)
            && found.is_empty()
        {
            let text_ =
                format!("Nothing in {} is new to the library.", folder_text(&self.editor, &self.dirs.import));
            let asker =
                if self.editor.flags.unattended { gz_ui::Asker::Unattended } else { gz_ui::Asker::Ui };
            if let Some(line) =
                gz_ui::dialog::notice(ui.ctx(), asker, Tone::Info, "Check New Textures", &text_)
            {
                self.editor.log.plain(line);
            }
            self.back_to_library();
        }
        let items = self.items();
        let [list_r, preview_r, details_r] = panes(ui, KIND);
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
            format!("Nothing in {} is new to the library.", folder_text(&self.editor, &self.dirs.import))
        } else {
            format!(
                "The library lists the .tga images in {}. To add one, put the image into {} and check for new textures.",
                folder_text(&self.editor, &self.dirs.textures),
                folder_text(&self.editor, &self.dirs.import)
            )
        };
        let empty = if items.is_none() {
            Empty { title: "Reading the library...", text: "", action: None }
        } else if import {
            Empty { title: "No new textures", text: &where_, action: None }
        } else if none {
            Empty {
                title: "No textures",
                text: &where_,
                action: (!self.read_only).then_some("Check New Textures"),
            }
        } else {
            Empty {
                title: "No textures of these kinds",
                text: "The Types filter hides every texture in the library.",
                action: None,
            }
        };
        let selected = self.selected.clone();
        let out = pane(ui, list_r, "texture_library.list.pane", false, false, |ui| {
            self.list.draw(ui, KIND, &keys, &rows, &selected, &empty)
        });
        if out.acted {
            // Check New Textures turns this tab into New Textures
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
        let names: Vec<&str> = if import { Vec::new() } else { keys.clone() };
        pane(ui, details_r, "texture_library.details", false, true, |ui| match &current {
            Some(item) => self.draw_properties(ui, item, &names),
            None => {
                state::empty_state(
                    ui,
                    Icon::Settings,
                    "No properties",
                    "Pick a texture to edit its description.",
                    None,
                );
            }
        });
        ui.allocate_rect(ui.max_rect(), Sense::hover());
    }

    fn status(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        let items = self.items().unwrap_or_default();
        let keys: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        let rows = shown(&keys, &|i| items.get(i).is_some_and(|t| self.visible(t)), &self.list.search);
        let changed = self.store.snapshot().edits.len();
        let mut line = format!("{} textures", items.len());
        if rows.len() != items.len() {
            line.push_str(&format!(" \u{b7} {} shown", rows.len()));
        }
        if changed > 0 && !self.import_mode() {
            line.push_str(&format!(" \u{b7} {changed} changed"));
        }
        let p = palette(ui.ctx());
        text::label(ui, &line, Font::Small, p.text_secondary, Clip::End);
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

    fn save(&mut self, _cx: &mut DocCx<'_>) -> Result<(), String> {
        self.queue_save()
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
