//! Tile pictures: asked for by key while a tile draws, made on the pools, uploaded on a later
//! frame, drawn when they are there (`GetThumb`, `MakeImageThumb`, `StoreThumb`).
//!
//! An image's picture is made for the size it is drawn at: the power of two at or over its
//! pixels, 32 to 512; drawn at another size, it is made again once the old one has shown a
//! frame. At most [`BUDGET`] pictures live; the coldest that no tile drew this frame goes, and
//! its texture is released [`RELEASE_AFTER`] frames later, after any draw list that still
//! names it. A folder change drops what has not started yet.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions};
use gz_editor::Editor;
use gz_fmt_tex::TileAlpha;
use gz_vfs::GameFiles;
use parking_lot::Mutex;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::catalog::Picture;
use crate::model::Source;

/// The most pictures kept.
pub const BUDGET: usize = 512;
/// Frames a texture outlives the last one that drew it.
pub const RELEASE_AFTER: u64 = 3;
/// Pictures made at once.
const IMAGES_AT_ONCE: usize = 6;
const MODELS_AT_ONCE: usize = 2;
/// A file larger than this gets no picture.
const MAX_BYTES: u64 = 64 << 20;
/// Models above it are not rendered for a tile.
const MAX_MODEL_BYTES: u64 = 16 << 20;

/// A picture's key: the source, the Editor category, the path in lower case.
pub type PicKey = (Source, usize, String);

/// A picture that is there.
pub struct Pic {
    tex: Option<TextureHandle>,
    pub aspect: f32,
    /// What it was made for (px) and what it got: an image is made again at another size.
    side: u32,
    edge: u32,
    full: bool,
    pub alpha: TileAlpha,
    /// The file's own size.
    pub source_size: Option<(u32, u32)>,
    used: u64,
    model: bool,
}

/// What a tile draws.
#[derive(Clone, Copy, Debug)]
pub struct Shown {
    pub tex: TextureId,
    pub aspect: f32,
    pub alpha: TileAlpha,
    pub source_size: Option<(u32, u32)>,
}

/// Where a picture's bytes are.
#[derive(Clone)]
pub enum Bytes {
    Disk(PathBuf),
    Game(Arc<GameFiles>, String),
}

struct Request {
    key: PicKey,
    side: u32,
    kind: Picture,
    name: String,
    from: Bytes,
}

struct Made {
    rgba: Vec<u8>,
    w: u32,
    h: u32,
    source_size: Option<(u32, u32)>,
    alpha: TileAlpha,
}

struct Done {
    key: PicKey,
    side: u32,
    generation: u64,
    model: bool,
    made: Option<Made>,
}

#[derive(Default)]
struct Shared {
    done: Mutex<Vec<Done>>,
    generation: AtomicU64,
    images: AtomicUsize,
    models: AtomicUsize,
}

/// The pictures of the browser. UI thread only, but for the results the jobs hand in.
#[derive(Default)]
pub struct Pictures {
    cache: FxHashMap<PicKey, Pic>,
    pending: FxHashSet<PicKey>,
    asked: Vec<Request>,
    dead: Vec<(u64, TextureHandle)>,
    shared: Arc<Shared>,
}

/// What a finished job pings.
fn ping(editor: &Editor) {
    editor.exec.wake().ping();
}

fn read(from: &Bytes, limit: u64) -> Option<Vec<u8>> {
    match from {
        Bytes::Disk(path) => {
            let meta = std::fs::metadata(path).ok()?;
            if meta.len() == 0 || meta.len() > limit {
                return None;
            }
            std::fs::read(path).ok()
        }
        Bytes::Game(files, path) => {
            let f = files.get(path)?;
            if f.size == 0 || f.size > limit {
                return None;
            }
            files.read_ref(f).ok()
        }
    }
}

impl Pictures {
    /// The jobs' results become textures; textures no draw list names any more go.
    pub fn begin_frame(&mut self, ctx: &Context, frame: u64) {
        self.dead.retain(|(at, _)| at + RELEASE_AFTER > frame);
        let done = std::mem::take(&mut *self.shared.done.lock());
        let generation = self.shared.generation.load(Ordering::Acquire);
        for d in done {
            self.pending.remove(&d.key);
            if d.generation != generation {
                continue;
            }
            let pic = match d.made {
                Some(m) => {
                    let image = ColorImage::from_rgba_unmultiplied([m.w as usize, m.h as usize], &m.rgba);
                    let tex = ctx.load_texture(format!("content_browser:{}", d.key.2), image, TextureOptions::LINEAR);
                    Pic {
                        tex: Some(tex),
                        aspect: m.w as f32 / m.h.max(1) as f32,
                        side: if d.model { 0 } else { d.side },
                        edge: m.w.max(m.h),
                        full: m.source_size.is_some_and(|(w, h)| m.w.max(m.h) >= w.max(h)),
                        alpha: m.alpha,
                        source_size: m.source_size,
                        used: frame,
                        model: d.model,
                    }
                }
                // nothing to show: the glyph stays, and nobody asks again
                None => Pic { tex: None, aspect: 1.0, side: 0, edge: 0, full: true, alpha: TileAlpha::Opaque, source_size: None, used: frame, model: d.model },
            };
            if let Some(old) = self.cache.insert(d.key, pic)
                && let Some(t) = old.tex
            {
                self.dead.push((frame, t));
            }
        }
        self.evict(frame);
    }

    fn evict(&mut self, frame: u64) {
        while self.cache.len() > BUDGET {
            // the coldest picture no tile drew this frame
            let victim = self.cache.iter().filter(|(_, p)| p.used < frame).min_by_key(|(_, p)| p.used).map(|(k, _)| k.clone());
            let Some(k) = victim else { break };
            if let Some(p) = self.cache.remove(&k)
                && let Some(t) = p.tex
            {
                self.dead.push((frame, t));
            }
        }
    }

    /// The picture of an entry drawn `px` pixels wide, when there is one; asks for it (or for a
    /// better one) otherwise. `from` says where its bytes are.
    pub fn get(&mut self, key: &PicKey, kind: Picture, name: &str, px: f32, frame: u64, from: impl FnOnce() -> Option<Bytes>) -> Option<Shown> {
        if kind == Picture::None {
            return None;
        }
        let want = if kind == Picture::Model { crate::model::picture_side(px).clamp(64, 256) } else { crate::model::picture_side(px) };
        let pending = self.pending.contains(key);
        if let Some(p) = self.cache.get_mut(key) {
            let remake = !p.model && p.side > 0 && p.used != frame && ((want > p.side && !p.full) || (want < p.side && p.edge > want));
            p.used = frame;
            let shown = p.tex.as_ref().map(|t| Shown { tex: t.id(), aspect: p.aspect, alpha: p.alpha, source_size: p.source_size });
            if remake && !pending && let Some(from) = from() {
                self.ask(key, want, kind, name, from);
            }
            return shown;
        }
        if !pending && let Some(from) = from() {
            self.ask(key, want, kind, name, from);
        }
        None
    }

    fn ask(&mut self, key: &PicKey, side: u32, kind: Picture, name: &str, from: Bytes) {
        if self.asked.iter().any(|r| &r.key == key) {
            return;
        }
        self.asked.push(Request { key: key.clone(), side, kind, name: name.to_owned(), from });
    }

    /// Starts what the frame asked for, as many as may run; the rest is asked again by the
    /// tiles of a later frame.
    pub(crate) fn dispatch(&mut self, editor: &Arc<Editor>, thumbs: &Arc<crate::thumbs::Thumbs>, gpu: Option<(&wgpu::Device, &wgpu::Queue)>, background: gz_render::Color) {
        let asked = std::mem::take(&mut self.asked);
        let generation = self.shared.generation.load(Ordering::Acquire);
        for r in asked {
            let shared = self.shared.clone();
            let ed = editor.clone();
            match r.kind {
                Picture::Image => {
                    if shared.images.load(Ordering::Acquire) >= IMAGES_AT_ONCE {
                        continue;
                    }
                    shared.images.fetch_add(1, Ordering::AcqRel);
                    self.pending.insert(r.key.clone());
                    let exec = editor.exec.clone();
                    editor.exec.spawn(async move {
                        let live = || shared.generation.load(Ordering::Acquire) == generation;
                        let mut made = None;
                        if live() {
                            let from = r.from.clone();
                            let bytes = exec.io(move || read(&from, MAX_BYTES)).await;
                            if let Some(bytes) = bytes
                                && live()
                            {
                                let ext = crate::icons::extension(&r.name).map(|e| e.trim_start_matches('.').to_ascii_lowercase());
                                let side = r.side;
                                made = exec
                                    .cpu(move || gz_fmt_tex::texture_tile(&bytes, ext.as_deref(), side).ok())
                                    .await
                                    .map(|t| Made { w: t.image.width, h: t.image.height, rgba: t.image.rgba, source_size: Some((t.source_width, t.source_height)), alpha: t.alpha });
                            }
                        }
                        shared.images.fetch_sub(1, Ordering::AcqRel);
                        shared.done.lock().push(Done { key: r.key, side: r.side, generation, model: false, made });
                        ping(&ed);
                    });
                }
                Picture::Model => {
                    let Some(renderer) = thumbs.renderer(editor, gpu, thumbs) else { continue };
                    if shared.models.load(Ordering::Acquire) >= MODELS_AT_ONCE {
                        continue;
                    }
                    shared.models.fetch_add(1, Ordering::AcqRel);
                    self.pending.insert(r.key.clone());
                    let thumbs = thumbs.clone();
                    editor.exec.spawn_io(move || {
                        let live = shared.generation.load(Ordering::Acquire) == generation;
                        let made = if live {
                            read(&r.from, MAX_MODEL_BYTES).and_then(|bytes| thumbs.render(&renderer, &r.name, &bytes, r.side, background).ok()).map(|img| Made {
                                w: img.width,
                                h: img.height,
                                rgba: img.rgba.clone(),
                                source_size: None,
                                alpha: TileAlpha::Opaque,
                            })
                        } else {
                            None
                        };
                        shared.models.fetch_sub(1, Ordering::AcqRel);
                        shared.done.lock().push(Done { key: r.key, side: r.side, generation, model: true, made });
                        ping(&ed);
                    });
                }
                Picture::None => {}
            }
        }
    }

    /// The folder or the source changed: what has not started is dropped, what runs is thrown
    /// away when it ends. The pictures made stay.
    pub fn cancel(&mut self) {
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.pending.clear();
        self.asked.clear();
    }

    /// Everything goes: another project, a listing made again.
    pub fn forget_all(&mut self) {
        self.cancel();
        let frame = self.cache.values().map(|p| p.used).max().unwrap_or(0);
        for (_, p) in self.cache.drain() {
            if let Some(t) = p.tex {
                self.dead.push((frame, t));
            }
        }
    }

    /// One entry's picture goes (its file changed).
    pub fn forget(&mut self, key: &PicKey, frame: u64) {
        if let Some(p) = self.cache.remove(key)
            && let Some(t) = p.tex
        {
            self.dead.push((frame, t));
        }
    }
}
