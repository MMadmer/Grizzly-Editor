//! The game's side of the PDA preview, read once per project and game link and kept for the
//! session (dropped with the project, or when the link changes, or on Reload): the texture ids
//! of every `textures_descr`, the sheets they cut from (decoded on the pools, uploaded once),
//! the bitmap fonts, the device of `pda.xml` / `pda_16.xml` and the level maps.
//!
//! A frame only asks: what is not there yet is requested, loaded off the UI thread, and the
//! window is woken when it arrives. Nothing here reads a file on the caller's thread.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{ColorImage, Context, Rect, TextureHandle, TextureId, TextureOptions, pos2};
use gz_base::Rev;
use gz_editor::{Editor, ProjectSession};
use gz_fmt_xml::sheets::TextureSheets;
use gz_game::{Bundled, GameLink};
use gz_vfs::GameFiles;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::bitmap::{GlyphTable, font_texture, parse_glyphs};
use crate::gamefile::{Device, LevelMap, parse_level_map};

/// Where a texture description came from: where its sheet is looked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SheetFrom {
    Game,
    Project,
    Bundled,
}

/// Every texture id of the game's descriptions, Refined's sheet when the game lacks it, and the
/// project's own - later ones redefine an id, the way the game lets a mod redefine one.
pub struct TexIds {
    sheets: TextureSheets,
    game_files: u32,
    bundled_files: u32,
}

impl TexIds {
    /// The sheet (textures-relative), where it came from and the piece of it, in its pixels.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<(&str, SheetFrom, [f32; 4])> {
        let e = self.sheets.get(id)?;
        let from = if e.file < self.game_files {
            SheetFrom::Game
        } else if e.file < self.game_files + self.bundled_files {
            SheetFrom::Bundled
        } else {
            SheetFrom::Project
        };
        Some((&e.sheet, from, e.rect))
    }
}

/// A sheet on the GPU and its size in pixels.
#[derive(Clone)]
pub struct Sheet {
    pub id: TextureId,
    pub w: f32,
    pub h: f32,
    _handle: TextureHandle,
}

impl Sheet {
    /// The uv rectangle of a piece of the sheet given in its pixels.
    #[must_use]
    pub fn uv(&self, x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_max(pos2(x / self.w, y / self.h), pos2((x + w) / self.w, (y + h) / self.h))
    }
}

/// A font of the game: its glyph table and the sheet it draws from.
#[derive(Clone, Debug)]
pub struct BitmapFont {
    pub glyphs: GlyphTable,
    /// Textures-relative, lower case.
    pub sheet: String,
}

/// What a lookup found.
#[derive(Clone)]
pub enum Look<T> {
    /// Asked for; it arrives on a later frame.
    Loading,
    /// The game would not find it either.
    Missing,
    Ready(T),
}

enum Slot<T> {
    Loading,
    Ready(Option<T>),
}

impl<T: Clone> Slot<T> {
    fn look(&self) -> Look<T> {
        match self {
            Slot::Loading => Look::Loading,
            Slot::Ready(None) => Look::Missing,
            Slot::Ready(Some(v)) => Look::Ready(v.clone()),
        }
    }
}

#[derive(Default)]
struct Cache {
    /// The game link's revision the cache was read under; `None` without a link.
    key: Option<Option<Rev>>,
    /// Moves on every reset: a load of an older generation is dropped when it comes back.
    generation: u64,
    tex_ids: Option<Slot<Arc<TexIds>>>,
    sheets: FxHashMap<String, Slot<Sheet>>,
    fonts: FxHashMap<String, Slot<Arc<BitmapFont>>>,
    device: Option<Slot<Arc<Device>>>,
    maps: FxHashMap<String, Slot<Arc<LevelMap>>>,
}

impl Cache {
    fn reset(&mut self) {
        let generation = self.generation + 1;
        *self = Cache { key: self.key, generation, ..Cache::default() };
    }
}

/// The preview's game assets of the open project (`session.services`).
pub struct PdaAssets {
    root: PathBuf,
    cache: Mutex<Cache>,
    ctx: Mutex<Option<Context>>,
}

/// A sheet's key: textures-relative, lower case, `\`, no `.dds` (`Sheet`).
#[must_use]
pub fn sheet_key(rel: &str) -> String {
    let mut k = rel.to_ascii_lowercase().replace('/', "\\");
    if k.len() > 4 && k.ends_with(".dds") {
        k.truncate(k.len() - 4);
    }
    k
}

fn base_name(path: &str) -> &str {
    let leaf = path.rsplit(['\\', '/']).next().unwrap_or(path);
    match leaf.rfind('.') {
        Some(dot) if dot > 0 => leaf.get(..dot).unwrap_or(leaf),
        _ => leaf,
    }
}

async fn game_files(editor: &Editor) -> Option<Arc<GameFiles>> {
    GameLink::of(editor).ok()?.files().await.ok()
}

fn read_game(files: Option<&GameFiles>, rel: &str) -> Option<Vec<u8>> {
    let files = files?;
    if !files.exists(rel) {
        return None;
    }
    files.read(rel).ok()
}

fn cp1251(bytes: &[u8]) -> String {
    gz_base::text::cp1251_to_utf8(bytes).into_owned()
}

/// The sheet's pixels: the project's `gamedata\textures`, the game's, and for Refined's own
/// sheet the editor's bundled copy (`Sheet`). Blocking.
fn sheet_bytes(root: &Path, files: Option<&GameFiles>, bundled_dir: &Path, key: &str, from: SheetFrom) -> Option<Vec<u8>> {
    let project = root.join("gamedata").join("textures").join(format!("{key}.dds"));
    if project.is_file()
        && let Ok(b) = std::fs::read(&project)
    {
        return Some(b);
    }
    if let Some(b) = read_game(files, &format!("textures\\{key}.dds")) {
        return Some(b);
    }
    if from == SheetFrom::Bundled {
        let bundled = bundled_dir.join(format!("{}.dds", base_name(key)));
        if bundled.is_file() {
            return std::fs::read(bundled).ok();
        }
    }
    None
}

impl PdaAssets {
    /// The assets of the open project's session, made on first use.
    #[must_use]
    pub fn of_session(session: &ProjectSession) -> Arc<Self> {
        session.services.get_or_init(|| PdaAssets { root: session.root.clone(), cache: Mutex::new(Cache::default()), ctx: Mutex::new(None) })
    }

    /// The window's context: what a decoded sheet is uploaded through.
    pub fn set_ctx(&self, ctx: &Context) {
        let mut slot = self.ctx.lock();
        if slot.is_none() {
            *slot = Some(ctx.clone());
        }
    }

    /// Drops everything when the game link changed since the cache was read.
    pub fn sync(&self, editor: &Editor) {
        let key = GameLink::of(editor).ok().map(|l| l.rev());
        let mut c = self.cache.lock();
        if c.key != Some(key) {
            c.reset();
            c.key = Some(key);
        }
    }

    /// Reload: the game's textures, fonts, device and maps are read again on their next use.
    pub fn invalidate(&self) {
        self.cache.lock().reset();
    }

    fn wake(&self, editor: &Editor) {
        if let Some(ctx) = self.ctx.lock().as_ref() {
            ctx.request_repaint();
        }
        editor.exec.wake().ping();
    }

    // ---- texture ids ----------------------------------------------------------------------

    fn tex_ids(self: &Arc<Self>, editor: &Arc<Editor>) -> Option<Arc<TexIds>> {
        let generation = {
            let mut c = self.cache.lock();
            match &c.tex_ids {
                Some(Slot::Ready(v)) => return v.clone(),
                Some(Slot::Loading) => return None,
                None => {
                    c.tex_ids = Some(Slot::Loading);
                    c.generation
                }
            }
        };
        let (me, ed) = (self.clone(), editor.clone());
        let _task = editor.exec.spawn(async move {
            let ids = me.load_tex_ids(&ed).await;
            {
                let mut c = me.cache.lock();
                if c.generation == generation {
                    c.tex_ids = Some(Slot::Ready(Some(Arc::new(ids))));
                }
            }
            me.wake(&ed);
        });
        None
    }

    async fn load_tex_ids(&self, editor: &Arc<Editor>) -> TexIds {
        let files = game_files(editor).await;
        let game_sheets = match GameLink::of(editor) {
            Ok(link) => link.texture_sheets().await.ok(),
            Err(_) => None,
        };
        // a game without Refined's PDA support still previews with the editor's copy of the sheet
        let need_bundled = !game_sheets.as_ref().is_some_and(|s| s.get("ui_xms_pda_white").is_some());
        let bundled = need_bundled.then(|| Bundled::of(editor).path("configs\\pda\\ui_xms_pda.xml"));
        let dir = self.root.join("gamedata").join("configs").join("ui").join("textures_descr");
        let exec = editor.exec.clone();
        editor
            .exec
            .io(move || {
                let mut texts: Vec<String> = Vec::new();
                if let Some(files) = &files {
                    let paths: Vec<&str> = files.find("configs/ui/textures_descr", "xml").into_iter().map(|f| f.path).collect();
                    let bytes = exec.io_pool().install(|| files.read_many(&paths));
                    texts.extend(bytes.into_iter().filter_map(Result::ok).map(|b| gz_fmt_xml::decode(&b).into_owned()));
                }
                let game_files = u32::try_from(texts.len()).unwrap_or(u32::MAX);
                let mut bundled_files = 0;
                if let Some(path) = bundled
                    && let Ok(b) = std::fs::read(path)
                {
                    texts.push(gz_fmt_xml::decode(&b).into_owned());
                    bundled_files = 1;
                }
                let mut own: Vec<PathBuf> = std::fs::read_dir(&dir)
                    .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("xml"))).collect())
                    .unwrap_or_default();
                own.sort();
                texts.extend(own.iter().filter_map(|p| std::fs::read(p).ok()).map(|b| gz_fmt_xml::decode(&b).into_owned()));
                let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
                TexIds { sheets: TextureSheets::build(&refs), game_files, bundled_files }
            })
            .await
    }

    // ---- sheets ---------------------------------------------------------------------------

    /// A sheet by its textures-relative path; the first ask decides where it is looked for.
    pub fn sheet(self: &Arc<Self>, editor: &Arc<Editor>, rel: &str, from: SheetFrom) -> Look<Sheet> {
        let key = sheet_key(rel);
        let generation = {
            let mut c = self.cache.lock();
            if let Some(slot) = c.sheets.get(&key) {
                return slot.look();
            }
            c.sheets.insert(key.clone(), Slot::Loading);
            c.generation
        };
        let Some(ctx) = self.ctx.lock().clone() else {
            self.cache.lock().sheets.remove(&key);
            return Look::Loading;
        };
        let (me, ed) = (self.clone(), editor.clone());
        let _task = editor.exec.spawn(async move {
            let files = game_files(&ed).await;
            let bundled_dir = Bundled::of(&ed).path("configs\\pda");
            let root = me.root.clone();
            let k = key.clone();
            let bytes = ed.exec.io(move || sheet_bytes(&root, files.as_deref(), &bundled_dir, &k, from)).await;
            let name = format!("pda:{key}");
            let made = match bytes {
                Some(bytes) => {
                    ed.exec
                        .cpu(move || {
                            let img = gz_fmt_tex::read_dds_level(&bytes, 0, 0).ok()?;
                            let size = [usize::try_from(img.width).ok()?, usize::try_from(img.height).ok()?];
                            let color = ColorImage::from_rgba_unmultiplied(size, &img.rgba);
                            let handle = ctx.load_texture(name, color, TextureOptions::LINEAR);
                            Some(Sheet { id: handle.id(), w: img.width as f32, h: img.height as f32, _handle: handle })
                        })
                        .await
                }
                None => None,
            };
            {
                let mut c = me.cache.lock();
                if c.generation == generation {
                    c.sheets.insert(key, Slot::Ready(made));
                }
            }
            me.wake(&ed);
        });
        Look::Loading
    }

    /// A texture id of the descriptions - or, not being one, a texture file (`Tex`): its sheet
    /// and uv rectangle.
    pub fn tex(self: &Arc<Self>, editor: &Arc<Editor>, id: &str) -> Look<(TextureId, Rect)> {
        if id.is_empty() {
            return Look::Missing;
        }
        let Some(ids) = self.tex_ids(editor) else { return Look::Loading };
        if let Some((sheet, from, [x, y, w, h])) = ids.get(id) {
            return match self.sheet(editor, sheet, from) {
                Look::Ready(s) => Look::Ready((s.id, s.uv(x, y, w, h))),
                Look::Loading => Look::Loading,
                Look::Missing => Look::Missing,
            };
        }
        match self.sheet(editor, id, SheetFrom::Game) {
            Look::Ready(s) => Look::Ready((s.id, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)))),
            Look::Loading => Look::Loading,
            Look::Missing => Look::Missing,
        }
    }

    // ---- fonts ----------------------------------------------------------------------------

    /// A font of the game and its sheet (`FontOf`); `Missing` without a linked game or when it
    /// does not read - the editor's own font stands in then.
    pub fn font(self: &Arc<Self>, editor: &Arc<Editor>, name: &str) -> Look<(Arc<BitmapFont>, Sheet)> {
        let key = if name.is_empty() { "letterica18".to_owned() } else { name.to_owned() };
        let (font, spawn) = {
            let mut c = self.cache.lock();
            if let Some(slot) = c.fonts.get(&key) {
                (slot.look(), None)
            } else {
                c.fonts.insert(key.clone(), Slot::Loading);
                (Look::Loading, Some(c.generation))
            }
        };
        if let Some(generation) = spawn {
            self.spawn_font(editor, key, generation);
        }
        match font {
            Look::Ready(f) => match self.sheet(editor, &f.sheet, SheetFrom::Game) {
                Look::Ready(s) => Look::Ready((f, s)),
                Look::Loading => Look::Loading,
                Look::Missing => Look::Missing,
            },
            Look::Loading => Look::Loading,
            Look::Missing => Look::Missing,
        }
    }

    fn spawn_font(self: &Arc<Self>, editor: &Arc<Editor>, key: String, generation: u64) {
        let (me, ed) = (self.clone(), editor.clone());
        let _task = editor.exec.spawn(async move {
            let files = game_files(&ed).await;
            let name = key.clone();
            let font = ed
                .exec
                .io(move || {
                    let files = files?;
                    let fonts = cp1251(&read_game(Some(&files), "configs\\fonts.ltx")?);
                    let tex = font_texture(&fonts, &name)?;
                    let ini = cp1251(&read_game(Some(&files), &format!("textures\\{tex}.ini"))?);
                    let glyphs = parse_glyphs(&ini)?;
                    (glyphs.height > 0.0).then(|| Arc::new(BitmapFont { glyphs, sheet: tex }))
                })
                .await;
            {
                let mut c = me.cache.lock();
                if c.generation == generation {
                    c.fonts.insert(key, Slot::Ready(font));
                }
            }
            me.wake(&ed);
        });
    }

    // ---- the device -----------------------------------------------------------------------

    async fn load_device(editor: &Editor) -> Arc<Device> {
        let files = game_files(editor).await;
        editor
            .exec
            .io(move || {
                let mut d = Device::default();
                for (wide, rel) in [(false, "configs\\ui\\pda.xml"), (true, "configs\\ui\\pda_16.xml")] {
                    if let Some(bytes) = read_game(files.as_deref(), rel) {
                        d.apply_xml(wide, &gz_fmt_xml::decode(&bytes));
                    }
                }
                Arc::new(d)
            })
            .await
    }

    /// The device (`Device`), or `None` the frames before it is read.
    pub fn device(self: &Arc<Self>, editor: &Arc<Editor>) -> Option<Arc<Device>> {
        let generation = {
            let mut c = self.cache.lock();
            match &c.device {
                Some(Slot::Ready(v)) => return v.clone(),
                Some(Slot::Loading) => return None,
                None => {
                    c.device = Some(Slot::Loading);
                    c.generation
                }
            }
        };
        let (me, ed) = (self.clone(), editor.clone());
        let _task = editor.exec.spawn(async move {
            let d = Self::load_device(&ed).await;
            {
                let mut c = me.cache.lock();
                if c.generation == generation {
                    c.device = Some(Slot::Ready(Some(d)));
                }
            }
            me.wake(&ed);
        });
        None
    }

    /// The device, read now when it is not there yet: what an MCP answer waits for.
    pub async fn device_wait(self: &Arc<Self>, editor: &Arc<Editor>) -> Arc<Device> {
        if let Some(Slot::Ready(Some(d))) = &self.cache.lock().device {
            return d.clone();
        }
        let generation = self.cache.lock().generation;
        let d = Self::load_device(editor).await;
        let mut c = self.cache.lock();
        if c.generation == generation {
            c.device = Some(Slot::Ready(Some(d.clone())));
        }
        d
    }

    // ---- level maps -----------------------------------------------------------------------

    async fn load_map(editor: &Editor, level: String) -> Option<Arc<LevelMap>> {
        if level.is_empty() {
            return None;
        }
        let files = game_files(editor).await?;
        editor
            .exec
            .io(move || {
                let ltx = cp1251(&read_game(Some(&files), &format!("levels\\{level}\\level.ltx"))?);
                parse_level_map(&ltx).map(Arc::new)
            })
            .await
    }

    /// A level's map (`LevelMap`).
    pub fn level_map(self: &Arc<Self>, editor: &Arc<Editor>, level: &str) -> Look<Arc<LevelMap>> {
        if level.is_empty() {
            return Look::Missing;
        }
        let generation = {
            let mut c = self.cache.lock();
            if let Some(slot) = c.maps.get(level) {
                return slot.look();
            }
            c.maps.insert(level.to_owned(), Slot::Loading);
            c.generation
        };
        let (me, ed, key) = (self.clone(), editor.clone(), level.to_owned());
        let _task = editor.exec.spawn(async move {
            let m = Self::load_map(&ed, key.clone()).await;
            {
                let mut c = me.cache.lock();
                if c.generation == generation {
                    c.maps.insert(key, Slot::Ready(m));
                }
            }
            me.wake(&ed);
        });
        Look::Loading
    }

    /// A level's map, read now when it is not there yet.
    pub async fn level_map_wait(self: &Arc<Self>, editor: &Arc<Editor>, level: &str) -> Option<Arc<LevelMap>> {
        if level.is_empty() {
            return None;
        }
        if let Some(Slot::Ready(m)) = self.cache.lock().maps.get(level) {
            return m.clone();
        }
        let generation = self.cache.lock().generation;
        let m = Self::load_map(editor, level.to_owned()).await;
        let mut c = self.cache.lock();
        if c.generation == generation {
            c.maps.insert(level.to_owned(), Slot::Ready(m.clone()));
        }
        m
    }
}
