//! The chooser's pictures: a model drawn by a renderer of the chooser's own, a texture decoded at
//! the size it is shown at. Asked for by name while a row or the preview draws, made on the pools,
//! taken into the GPU a few a frame, drawn when they are there. A frame never reads, decodes or
//! renders anything.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use egui::{ColorImage, Context, TextureHandle, TextureId, TextureOptions};
use glam::{Mat4, Vec3};
use gz_editor::Editor;
use gz_render::{
    AssetBytes, AssetRef, Camera, Color, DrawList, Instance, MeshData, Projection, Renderer, ThumbnailCache,
    View,
};
use parking_lot::Mutex;
use rustc_hash::{FxHashMap, FxHashSet};

/// What a picture shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PicKind {
    /// A game model, `meshes\<name>.ogf`.
    Visual,
    /// A library object, `<name>.object`.
    Object,
    /// A game texture, `textures\<name>.dds`.
    Texture,
}

/// A picture's key: what, its name, its size in px.
type Key = (PicKind, String, u32);

/// The most pictures kept.
const BUDGET: usize = 256;
/// Pictures taken into the GPU in one frame.
const UPLOADS_PER_FRAME: usize = 3;
const MODELS_AT_ONCE: usize = 2;
const TEXTURES_AT_ONCE: usize = 4;
/// How long a model's picture waits for its textures.
const WAIT: Duration = Duration::from_secs(3);
/// Files larger than this get no picture.
const MAX_BYTES: u64 = 32 << 20;

struct Made {
    key: Key,
    rgba: Option<(u32, u32, Vec<u8>)>,
    /// What a model's picture was drawn on; a picture of another theme's is drawn again.
    backdrop: Option<[u8; 3]>,
}

/// The chooser's renderer and its pictures, an `Editor` service.
#[derive(Default)]
pub struct Pics {
    renderer: Mutex<Option<Arc<Renderer>>>,
    starting: AtomicBool,
    failed: AtomicBool,
    cache: ThumbnailCacheSlot,
    done: Mutex<Vec<Made>>,
    models: AtomicUsize,
    textures: AtomicUsize,
    /// UI thread only.
    shown: Mutex<Shown>,
    /// What a model's picture stands on: the theme's overlay colour, as the last frame had it.
    backdrop: Mutex<[u8; 3]>,
}

#[derive(Default)]
struct Shown {
    textures: FxHashMap<Key, (Option<TextureHandle>, u64)>,
    pending: FxHashSet<Key>,
}

/// The renderer's own cache of model pictures, made with the service.
struct ThumbnailCacheSlot(ThumbnailCache);

impl Default for ThumbnailCacheSlot {
    fn default() -> Self {
        Self(ThumbnailCache::new(32 << 20))
    }
}

impl Pics {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Pics> {
        editor.services.get_or_init(Pics::default)
    }

    /// The jobs' results become textures, a few a frame; the coldest go past the budget.
    pub fn begin_frame(&self, ctx: &Context, frame: u64) {
        // another theme: the model pictures are drawn again on its overlay colour
        let o = gz_ui::palette(ctx).overlay;
        let now = [o.r(), o.g(), o.b()];
        if std::mem::replace(&mut *self.backdrop.lock(), now) != now {
            self.shown.lock().textures.retain(|k, _| k.0 == PicKind::Texture);
        }
        let mut done = self.done.lock();
        let take = done.len().min(UPLOADS_PER_FRAME);
        let mut made: Vec<Made> = done.drain(..take).collect();
        drop(done);
        // a picture drawn on the theme before: asked again by the frame that shows it
        made.retain(|m| {
            let stale = m.backdrop.is_some_and(|b| b != now);
            if stale {
                self.shown.lock().pending.remove(&m.key);
            }
            !stale
        });
        if !made.is_empty() {
            ctx.request_repaint();
        }
        let mut shown = self.shown.lock();
        for m in made {
            shown.pending.remove(&m.key);
            let tex = m.rgba.map(|(w, h, rgba)| {
                let image = ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
                ctx.load_texture(format!("chooser:{}", m.key.1), image, TextureOptions::LINEAR)
            });
            shown.textures.insert(m.key, (tex, frame));
        }
        while shown.textures.len() > BUDGET {
            let cold =
                shown.textures.iter().filter(|(_, (_, used))| *used < frame).min_by_key(|(_, (_, u))| *u);
            let Some(k) = cold.map(|(k, _)| k.clone()) else { break };
            shown.textures.remove(&k);
        }
    }

    /// The picture of `name` at `px`, when it is there; asks for it otherwise.
    pub fn get(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        gpu: Option<(&wgpu::Device, &wgpu::Queue)>,
        kind: PicKind,
        name: &str,
        px: u32,
        frame: u64,
    ) -> Option<TextureId> {
        let side = px.next_power_of_two().clamp(32, 512);
        let key: Key = (kind, name.to_owned(), side);
        {
            let mut shown = self.shown.lock();
            if let Some((tex, used)) = shown.textures.get_mut(&key) {
                *used = frame;
                return tex.as_ref().map(TextureHandle::id);
            }
            if shown.pending.contains(&key) {
                return None;
            }
        }
        let started = match kind {
            PicKind::Texture => self.start_texture(editor, key.clone()),
            PicKind::Visual | PicKind::Object => self.start_model(editor, gpu, key.clone()),
        };
        if started {
            self.shown.lock().pending.insert(key);
        }
        None
    }

    fn start_texture(self: &Arc<Self>, editor: &Arc<Editor>, key: Key) -> bool {
        if self.textures.load(Ordering::Acquire) >= TEXTURES_AT_ONCE {
            return false;
        }
        self.textures.fetch_add(1, Ordering::AcqRel);
        let (me, ed) = (self.clone(), editor.clone());
        editor.exec.spawn(async move {
            let rel = format!("textures/{}.dds", key.1.replace('\\', "/").to_lowercase());
            let bytes = game_bytes(&ed, &rel).await;
            let side = key.2;
            let rgba = match bytes {
                Some(b) => ed
                    .exec
                    .cpu(move || gz_fmt_tex::texture_tile(&b, Some("dds"), side).ok())
                    .await
                    .map(|t| (t.image.width, t.image.height, t.image.rgba)),
                None => None,
            };
            me.textures.fetch_sub(1, Ordering::AcqRel);
            me.done.lock().push(Made { key, rgba, backdrop: None });
            ed.exec.wake().ping();
        });
        true
    }

    fn start_model(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        gpu: Option<(&wgpu::Device, &wgpu::Queue)>,
        key: Key,
    ) -> bool {
        let Some(renderer) = self.renderer(editor, gpu) else { return false };
        if self.models.load(Ordering::Acquire) >= MODELS_AT_ONCE {
            return false;
        }
        self.models.fetch_add(1, Ordering::AcqRel);
        let (me, ed) = (self.clone(), editor.clone());
        let backdrop = *self.backdrop.lock();
        editor.exec.spawn(async move {
            let (kind, name, side) = (key.0, key.1.clone(), key.2);
            let bytes = match kind {
                PicKind::Object => object_bytes(&ed, &name).await,
                _ => game_bytes(&ed, &format!("meshes/{}.ogf", name.replace('\\', "/").to_lowercase())).await,
            };
            let me2 = me.clone();
            let rgba = match bytes {
                Some(b) => ed
                    .exec
                    .io(move || me2.render(&renderer, kind, &name, &b, side, backdrop).ok())
                    .await
                    .map(|img| (img.width, img.height, img.rgba.clone())),
                None => None,
            };
            me.models.fetch_sub(1, Ordering::AcqRel);
            me.done.lock().push(Made { key, rgba, backdrop: Some(backdrop) });
            ed.exec.wake().ping();
        });
        true
    }

    /// The renderer, when it is up; the first call starts it on the CPU pool.
    fn renderer(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        gpu: Option<(&wgpu::Device, &wgpu::Queue)>,
    ) -> Option<Arc<Renderer>> {
        if let Some(r) = self.renderer.lock().clone() {
            return Some(r);
        }
        if self.failed.load(Ordering::Acquire) {
            return None;
        }
        let (device, queue) = gpu?;
        if self.starting.swap(true, Ordering::SeqCst) {
            return None;
        }
        let (device, queue) = (device.clone(), queue.clone());
        let exec = editor.exec.clone();
        let gate = gz_app::gpu_gate(editor);
        let weak: Weak<Editor> = Arc::downgrade(editor);
        let me = self.clone();
        editor.exec.spawn_cpu(move || {
            match Renderer::new(device, queue, gate, exec.clone()) {
                Ok(r) => {
                    let loader = weak.clone();
                    r.assets().set_loader(Some(Arc::new(move |what: AssetRef<'_>| match what {
                        AssetRef::Texture(name) => match loader.upgrade() {
                            Some(editor) => texture_asset(&editor, name),
                            None => Err("the editor is closing".to_owned()),
                        },
                        other => Err(format!("{other:?} is not loaded for a picture")),
                    })));
                    *me.renderer.lock() = Some(Arc::new(r));
                    // the window stops the render thread while its loop still runs
                    if let Some(editor) = weak.upgrade() {
                        let owner = me.clone();
                        gz_app::GpuOwners::of(&editor).on_exit("the chooser's renderer", move || {
                            if let Some(r) = owner.renderer.lock().take() {
                                r.shutdown();
                            }
                        });
                    }
                }
                Err(_) => me.failed.store(true, Ordering::Release),
            }
            me.starting.store(false, Ordering::SeqCst);
            exec.wake().ping();
        });
        None
    }

    /// A model's picture: a three-quarter view from above on the overlay colour. Blocks while the
    /// model's textures load: the IO pool only.
    fn render(
        &self,
        renderer: &Renderer,
        kind: PicKind,
        name: &str,
        bytes: &[u8],
        side: u32,
        backdrop: [u8; 3],
    ) -> Result<Arc<gz_render::Image>, String> {
        let table = renderer.assets().table();
        let mesh = if kind == PicKind::Object {
            let o = gz_fmt_object::Object::read(bytes).map_err(|e| e.to_string())?;
            MeshData::from_object(&o, &table).map_err(|e| e.to_string())?
        } else {
            let v = gz_fmt_ogf::Visual::read(bytes).map_err(|e| e.to_string())?;
            MeshData::from_visual(&v, &table).map_err(|e| e.to_string())?
        };
        let mesh_key = key_of(kind, name, bytes.len());
        // the backdrop is part of the picture, not of the mesh
        let [r, g, b] = backdrop;
        let key = mesh_key ^ (u64::from(r) << 40 | u64::from(g) << 32 | u64::from(b) << 24);
        let (center, radius) = mesh.bounds.sphere();
        let radius = if radius.is_finite() && radius > 1e-4 { radius } else { 1.0 };
        let handle = renderer.assets().add_mesh(&format!("chooser\\{mesh_key:016x}"), mesh);
        let fov = 45f32.to_radians();
        let dist = radius / (fov * 0.5).sin() * 1.12;
        let eye = center + Vec3::new(1.0, 0.8, -1.0).normalize() * dist;
        let camera = Camera::look_at(eye, center).with_projection(Projection::Perspective {
            fov_y: fov,
            near: (dist - radius * 1.5).max(0.01),
            far: dist + radius * 2.0,
        });
        self.cache
            .0
            .get_or_render(renderer, key, [side, side], WAIT, move |_| {
                let mut list = DrawList::new();
                list.push(Instance::new(handle, Mat4::IDENTITY));
                let mut view = View::new(camera, [side, side]);
                // the overlay token's colour, the dialog the picture stands on
                view.background = Color::rgb(r, g, b);
                (list, view)
            })
            .map_err(|e| e.to_string())
    }
}

fn key_of(kind: PicKind, name: &str, len: usize) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};
    let mut h = BuildHasherDefault::<rustc_hash::FxHasher>::default().build_hasher();
    kind.hash(&mut h);
    name.to_lowercase().hash(&mut h);
    len.hash(&mut h);
    h.finish()
}

async fn game_bytes(editor: &Arc<Editor>, rel: &str) -> Option<Vec<u8>> {
    let files = gz_game::GameLink::of(editor).ok()?.files().await.ok()?;
    let rel = rel.to_owned();
    editor
        .exec
        .io(move || {
            let f = files.get(&rel)?;
            if f.size == 0 || f.size > MAX_BYTES {
                return None;
            }
            files.read_ref(f).ok()
        })
        .await
}

async fn object_bytes(editor: &Arc<Editor>, name: &str) -> Option<Vec<u8>> {
    let lib = editor.project_opt()?.services.get::<gz_scene::SceneService>()?.library.clone();
    let name = name.to_owned();
    editor.exec.io(move || std::fs::read(lib.file_of(&name)?).ok()).await
}

/// A texture a model names: the project's, the editor's, the linked game's.
fn texture_asset(editor: &Editor, name: &str) -> Result<AssetBytes, String> {
    let rel = name.replace('/', "\\");
    let mut disk = Vec::new();
    if let Some(root) = editor.project_root() {
        disk.push(root.join("gamedata\\textures").join(format!("{rel}.dds")));
    }
    disk.push(gz_project::library_root(editor).join("gamedata\\textures").join(format!("{rel}.dds")));
    for path in &disk {
        if let Ok(bytes) = std::fs::read(path) {
            return Ok(AssetBytes { bytes, ext: Some("dds".into()) });
        }
    }
    if let Ok((_, project)) = gz_project::Project::of(editor)
        && let Ok(files) = project.game_files()
        && let Ok(bytes) = files.read(&format!("textures\\{rel}.dds"))
    {
        return Ok(AssetBytes { bytes, ext: Some("dds".into()) });
    }
    Err(format!("the texture '{name}' is not in the project, the editor or the game"))
}
