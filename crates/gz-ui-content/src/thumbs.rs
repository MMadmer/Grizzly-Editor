//! A model's tile picture: rendered offscreen by a renderer of the browser's own, made on first
//! need, and kept by content in a `gz_render::ThumbnailCache`.
//!
//! The shot: a three-quarter view from above (the camera offset 1, 0.8, -1 from
//! the model's centre), 45 degrees, far enough for the bounding sphere and a little more, on the
//! colour of the tile's square.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use glam::{Mat4, Vec3};
use gz_editor::Editor;
use gz_project::Project;
use gz_render::{
    AssetBytes, AssetRef, Camera, Color, DrawList, Image, Instance, MeshData, Projection, Renderer,
    ThumbnailCache, View,
};
use parking_lot::Mutex;

/// The pictures kept, by bytes.
const BUDGET: usize = 64 << 20;
/// How long a picture waits for the model's textures.
const WAIT: Duration = Duration::from_secs(3);

/// The browser's thumbnail renderer and its pictures.
pub(crate) struct Thumbs {
    renderer: Mutex<Option<Arc<Renderer>>>,
    starting: AtomicBool,
    failed: Mutex<Option<String>>,
    cache: ThumbnailCache,
}

impl Default for Thumbs {
    fn default() -> Self {
        Self {
            renderer: Mutex::new(None),
            starting: AtomicBool::new(false),
            failed: Mutex::new(None),
            cache: ThumbnailCache::new(BUDGET),
        }
    }
}

/// Where a texture a model names is: the project's `gamedata\textures`, the editor's, the linked
/// game's index, the editor's `.tga` sources.
fn load_texture(editor: &Editor, name: &str) -> Result<AssetBytes, String> {
    let rel = name.replace('/', "\\");
    let mut disk: Vec<(PathBuf, &str)> = Vec::new();
    if let Some(root) = editor.project_root() {
        disk.push((root.join("gamedata\\textures").join(format!("{rel}.dds")), "dds"));
    }
    disk.push((
        gz_project::library_root(editor).join("gamedata\\textures").join(format!("{rel}.dds")),
        "dds",
    ));
    for (path, ext) in &disk {
        if let Ok(bytes) = std::fs::read(path) {
            return Ok(AssetBytes { bytes, ext: Some((*ext).into()) });
        }
    }
    if let Ok((_, project)) = Project::of(editor)
        && let Ok(files) = project.game_files()
        && let Ok(bytes) = files.read(&format!("textures\\{rel}.dds"))
    {
        return Ok(AssetBytes { bytes, ext: Some("dds".into()) });
    }
    let tga = gz_project::library_root(editor).join("rawdata\\textures").join(format!("{rel}.tga"));
    std::fs::read(&tga)
        .map(|bytes| AssetBytes { bytes, ext: Some("tga".into()) })
        .map_err(|_| format!("the texture '{name}' is not in the project, the editor or the game"))
}

impl Thumbs {
    /// The renderer, when it is up; the first call starts it on the CPU pool.
    pub(crate) fn renderer(
        &self,
        editor: &Arc<Editor>,
        gpu: Option<(&wgpu::Device, &wgpu::Queue)>,
        me: &Arc<Thumbs>,
    ) -> Option<Arc<Renderer>> {
        if let Some(r) = self.renderer.lock().clone() {
            return Some(r);
        }
        if self.failed.lock().is_some() {
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
        let weak_owner = weak.clone();
        let me = me.clone();
        editor.exec.spawn_cpu(move || {
            match Renderer::new(device, queue, gate, exec.clone()) {
                Ok(r) => {
                    r.assets().set_loader(Some(Arc::new(move |what: AssetRef<'_>| match what {
                        AssetRef::Texture(name) => match weak.upgrade() {
                            Some(editor) => load_texture(&editor, name),
                            None => Err("the editor is closing".to_owned()),
                        },
                        other => Err(format!("{other:?} is not loaded for a thumbnail")),
                    })));
                    *me.renderer.lock() = Some(Arc::new(r));
                    // the window stops the render thread while its loop still runs
                    if let Some(editor) = weak_owner.upgrade() {
                        let owner = me.clone();
                        gz_app::GpuOwners::of(&editor).on_exit("the Content Browser's renderer", move || {
                            let taken = owner.renderer.lock().take();
                            if let Some(r) = taken {
                                r.shutdown();
                            }
                        });
                    }
                }
                Err(e) => *me.failed.lock() = Some(e.to_string()),
            }
            me.starting.store(false, Ordering::SeqCst);
            exec.wake().ping();
        });
        None
    }

    /// The picture of a model file's bytes at `side` pixels. Blocks while the model's textures
    /// load: call it on the IO pool.
    pub(crate) fn render(
        &self,
        renderer: &Renderer,
        name: &str,
        bytes: &[u8],
        side: u32,
        background: Color,
    ) -> Result<Arc<Image>, String> {
        let table = renderer.assets().table();
        let object = crate::icons::extension(name).is_some_and(|e| e.eq_ignore_ascii_case(".object"));
        let mesh = if object {
            let o = gz_fmt_object::Object::read(bytes).map_err(|e| e.to_string())?;
            MeshData::from_object(&o, &table).map_err(|e| e.to_string())?
        } else {
            let v = gz_fmt_ogf::Visual::read(bytes).map_err(|e| e.to_string())?;
            MeshData::from_visual(&v, &table).map_err(|e| e.to_string())?
        };
        let key = content_key(name, bytes);
        let (center, radius) = mesh.bounds.sphere();
        let radius = if radius.is_finite() && radius > 1e-4 { radius } else { 1.0 };
        let handle = renderer.assets().add_mesh(&format!("content_browser\\{key:016x}"), mesh);
        let fov = 45f32.to_radians();
        let dist = radius / (fov * 0.5).sin() * 1.12;
        let eye = center + Vec3::new(1.0, 0.8, -1.0).normalize() * dist;
        let camera = Camera::look_at(eye, center).with_projection(Projection::Perspective {
            fov_y: fov,
            near: (dist - radius * 1.5).max(0.01),
            far: dist + radius * 2.0,
        });
        self.cache
            .get_or_render(renderer, key, [side, side], WAIT, move |_| {
                let mut list = DrawList::new();
                list.push(Instance::new(handle, Mat4::IDENTITY));
                let mut view = View::new(camera, [side, side]);
                view.background = background;
                (list, view)
            })
            .map_err(|e| e.to_string())
    }
}

/// What a picture is kept by: the name and the bytes, so a changed file is drawn anew.
fn content_key(name: &str, bytes: &[u8]) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};
    let mut h = BuildHasherDefault::<rustc_hash::FxHasher>::default().build_hasher();
    name.to_lowercase().hash(&mut h);
    bytes.len().hash(&mut h);
    // a sample of the bytes, not all of them: a model of 16 MB is not hashed whole
    for chunk in bytes.chunks(4096).step_by(16) {
        chunk.hash(&mut h);
    }
    h.finish()
}
