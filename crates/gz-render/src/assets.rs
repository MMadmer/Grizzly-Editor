//! Meshes, textures and materials, requested by handle and loaded off the frame.
//!
//! A handle is a slot that outlives what it holds: a mesh or a texture that is not loaded yet,
//! failed, or was evicted draws a placeholder (a grey box, a checker) and is (re)loaded by a
//! job - the file through the caller's [`AssetLoader`] on the IO pool, decoding and the upload
//! on the CPU pool. Caches are LRUs bounded by bytes; what the last packet used is never
//! evicted. Every change bumps [`Assets::generation`].

use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use gz_task::Executors;
use parking_lot::{Condvar, Mutex, RwLock};
use rustc_hash::FxHashMap;
use smol_str::SmolStr;

use crate::color::Color;
use crate::draw_list::DrawList;
use crate::material::{EditorMaterial, MaterialDesc, MaterialKey, MaterialTable, model_key, texture_key};
use crate::math::Aabb;
use crate::mesh::MeshData;
use crate::texture::{TextureData, level_layout};
use crate::{Result, invalid};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MeshHandle(pub(crate) u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureHandle(pub(crate) u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct MaterialHandle(pub(crate) u32);

/// What a mesh handle stands for.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MeshKey {
    /// A library object (`.object`), by its name in the library.
    Object(SmolStr),
    /// A compiled visual (`.ogf`), by its path under `meshes`.
    Visual(SmolStr),
    /// A mesh handed over with [`Assets::add_mesh`].
    Named(SmolStr),
}

impl MeshKey {
    fn normalized(self) -> Self {
        match self {
            Self::Object(n) => Self::Object(model_key(&n)),
            Self::Visual(n) => Self::Visual(model_key(&n)),
            Self::Named(n) => Self::Named(n),
        }
    }
}

/// What the renderer asks a loader for. Names are lower case with back slashes and carry no
/// extension: the loader knows where the project and the linked game keep them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetRef<'a> {
    /// A texture: a `.dds` of the game, or a `.tga` / `.png` source.
    Texture(&'a str),
    Object(&'a str),
    Visual(&'a str),
}

/// A file a loader found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssetBytes {
    pub bytes: Vec<u8>,
    /// The file's extension, without the dot: how a picture that is not a DDS is read.
    pub ext: Option<SmolStr>,
}

/// Where the bytes come from: the project and the linked game, supplied by the caller. Called on
/// the IO pool, many at once.
pub trait AssetLoader: Send + Sync {
    /// The bytes, or a sentence saying why there are none.
    fn load(&self, what: AssetRef<'_>) -> Result<AssetBytes, String>;
}

impl<F> AssetLoader for F
where
    F: Fn(AssetRef<'_>) -> Result<AssetBytes, String> + Send + Sync,
{
    fn load(&self, what: AssetRef<'_>) -> Result<AssetBytes, String> {
        self(what)
    }
}

/// How much the caches keep before the least recently used goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budgets {
    pub mesh_bytes: usize,
    pub texture_bytes: usize,
}

impl Default for Budgets {
    fn default() -> Self {
        Self { mesh_bytes: 768 << 20, texture_bytes: 1536 << 20 }
    }
}

/// Where a handle stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssetState {
    /// Not asked for yet, or evicted.
    Missing,
    Loading,
    Ready,
    Failed(SmolStr),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AssetStats {
    pub meshes: usize,
    pub textures: usize,
    pub materials: usize,
    pub mesh_bytes: usize,
    pub texture_bytes: usize,
    pub loading: usize,
    pub failed: usize,
}

/// What the packet builder knows of a loaded mesh.
#[derive(Debug)]
pub(crate) struct MeshInfo {
    pub bounds: Aabb,
    pub submeshes: Vec<SubInfo>,
    pub skinned: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct SubInfo {
    pub indices: Range<u32>,
    pub wire: Range<u32>,
    pub material: MaterialHandle,
}

/// A mesh on the GPU.
pub(crate) struct GpuMesh {
    pub vertices: wgpu::Buffer,
    pub indices: wgpu::Buffer,
    pub wire: wgpu::Buffer,
    pub info: Arc<MeshInfo>,
}

pub(crate) struct GpuTexture {
    _texture: wgpu::Texture,
    /// The picture, or a cube map's first face.
    pub view: wgpu::TextureView,
    /// A cube map's view as a cube.
    pub cube: Option<wgpu::TextureView>,
}

/// What a material is drawn as, for the packet builder.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct MaterialInfo {
    pub kind: EditorMaterial,
    pub two_sided: bool,
}

/// The renderer's side of the assets: the device and what a material bind group is made of.
pub(crate) struct AssetGpu {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub material_layout: wgpu::BindGroupLayout,
    pub sampler: wgpu::Sampler,
    pub white: Arc<GpuTexture>,
    pub checker: Arc<GpuTexture>,
    /// What a material without a cube map binds where a sky would sample one.
    pub white_cube: Arc<GpuTexture>,
    pub bc: bool,
    pub max_side: u32,
}

#[derive(Clone)]
enum State<T> {
    Missing,
    Loading,
    Ready(T),
    Failed(SmolStr),
}

impl<T> State<T> {
    fn public(&self) -> AssetState {
        match self {
            Self::Missing => AssetState::Missing,
            Self::Loading => AssetState::Loading,
            Self::Ready(_) => AssetState::Ready,
            Self::Failed(e) => AssetState::Failed(e.clone()),
        }
    }
}

#[derive(Clone)]
struct MeshReady {
    info: Arc<MeshInfo>,
    gpu: Option<Arc<GpuMesh>>,
}

struct Slot<K, T> {
    key: K,
    state: State<T>,
    bytes: usize,
    last_used: u64,
    /// Bumped on every change of state: what a material's bind group was built against.
    version: u64,
    /// Handed over by the caller: never evicted.
    pinned: bool,
}

impl<K, T> Slot<K, T> {
    fn new(key: K) -> Self {
        Self { key, state: State::Missing, bytes: 0, last_used: 0, version: 0, pinned: false }
    }
}

struct MatSlot {
    desc: MaterialDesc,
    base: Option<TextureHandle>,
    detail: Option<TextureHandle>,
    ubo: Option<wgpu::Buffer>,
    bind: Option<(Arc<wgpu::BindGroup>, (u64, u64))>,
}

#[derive(Default)]
struct Inner {
    meshes: Vec<Slot<MeshKey, MeshReady>>,
    mesh_ids: FxHashMap<MeshKey, u32>,
    textures: Vec<Slot<SmolStr, Option<Arc<GpuTexture>>>>,
    texture_ids: FxHashMap<SmolStr, u32>,
    materials: Vec<MatSlot>,
    material_ids: FxHashMap<MaterialKey, u32>,
    mesh_bytes: usize,
    texture_bytes: usize,
    loading: usize,
}

/// The renderer's meshes, textures and materials. `Send + Sync`; build packets with it from any
/// thread.
pub struct Assets {
    me: Weak<Assets>,
    inner: Mutex<Inner>,
    loaded: Condvar,
    gpu: Option<AssetGpu>,
    exec: Option<Executors>,
    loader: RwLock<Option<Arc<dyn AssetLoader>>>,
    table: RwLock<Arc<MaterialTable>>,
    budgets: Budgets,
    generation: AtomicU64,
    tick: AtomicU64,
    epoch: AtomicU64,
    on_change: RwLock<Option<Arc<dyn Fn() + Send + Sync>>>,
    placeholder: MeshHandle,
    default_material: MaterialHandle,
    skybox: MeshHandle,
}

const PLACEHOLDER: &str = "<placeholder>";
const SKYBOX: &str = "<skybox>";

impl Assets {
    /// Assets without a GPU and without loading: what tests and packet building ahead of a
    /// device use. Only meshes handed over with [`Assets::add_mesh`] are ever ready.
    #[must_use]
    pub fn headless(table: MaterialTable) -> Arc<Self> {
        Self::create(None, None, table, Budgets::default())
    }

    pub(crate) fn with_gpu(
        gpu: AssetGpu,
        exec: Executors,
        table: MaterialTable,
        budgets: Budgets,
    ) -> Arc<Self> {
        Self::create(Some(gpu), Some(exec), table, budgets)
    }

    fn create(
        gpu: Option<AssetGpu>,
        exec: Option<Executors>,
        table: MaterialTable,
        budgets: Budgets,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| {
            let mut a = Self {
                me: me.clone(),
                inner: Mutex::new(Inner::default()),
                loaded: Condvar::new(),
                gpu,
                exec,
                loader: RwLock::new(None),
                table: RwLock::new(Arc::new(table)),
                budgets,
                generation: AtomicU64::new(1),
                tick: AtomicU64::new(1),
                epoch: AtomicU64::new(1),
                on_change: RwLock::new(None),
                placeholder: MeshHandle(0),
                default_material: MaterialHandle(0),
                skybox: MeshHandle(0),
            };
            let grey = MaterialDesc::color(EditorMaterial::Opaque, Color::PLACEHOLDER);
            a.default_material = a.material(grey.clone());
            a.placeholder = a.add_mesh(PLACEHOLDER, MeshData::cube(grey));
            a.skybox = a.add_mesh(SKYBOX, MeshData::sky_box());
            a
        })
    }

    /// The half box a weather's sky is drawn on ([`MeshData::sky_box`]).
    pub(crate) fn skybox(&self) -> MeshHandle {
        self.skybox
    }

    /// Sets where the bytes come from. Everything loaded through the previous loader is dropped
    /// (a new project or a new linked game): handles stay valid and load again when drawn.
    pub fn set_loader(&self, loader: Option<Arc<dyn AssetLoader>>) {
        *self.loader.write() = loader;
        self.epoch.fetch_add(1, Ordering::AcqRel);
        {
            let mut inner = self.inner.lock();
            let inner = &mut *inner;
            for slot in inner.meshes.iter_mut().filter(|s| !s.pinned) {
                slot.state = State::Missing;
                inner.mesh_bytes -= slot.bytes;
                slot.bytes = 0;
                slot.version += 1;
            }
            for slot in inner.textures.iter_mut().filter(|s| !s.pinned) {
                slot.state = State::Missing;
                inner.texture_bytes -= slot.bytes;
                slot.bytes = 0;
                slot.version += 1;
            }
            for m in &mut inner.materials {
                m.bind = None;
            }
            inner.loading = 0;
        }
        self.changed();
    }

    /// Replaces the shader-to-material table; meshes loaded with the old one load again.
    pub fn set_table(&self, table: MaterialTable) {
        *self.table.write() = Arc::new(table);
        let loader = self.loader.read().clone();
        self.set_loader(loader);
    }

    #[must_use]
    pub fn table(&self) -> Arc<MaterialTable> {
        self.table.read().clone()
    }

    /// Called whenever an asset becomes ready or goes: the renderer's way to draw again.
    pub(crate) fn set_on_change(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *self.on_change.write() = Some(f);
    }

    fn changed(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.loaded.notify_all();
        let hook = self.on_change.read().clone();
        if let Some(f) = hook {
            f();
        }
    }

    /// Bumped by every change: a packet built before it may now draw something better.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub(crate) fn next_tick(&self) -> u64 {
        self.tick.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub(crate) fn placeholder(&self) -> MeshHandle {
        self.placeholder
    }

    pub(crate) fn default_material(&self) -> MaterialHandle {
        self.default_material
    }

    /// The handle of a mesh; nothing loads until it is drawn.
    pub fn mesh(&self, key: MeshKey) -> MeshHandle {
        let key = key.normalized();
        let mut inner = self.inner.lock();
        mesh_slot(&mut inner, key)
    }

    /// [`MeshKey::Object`].
    pub fn object(&self, name: &str) -> MeshHandle {
        self.mesh(MeshKey::Object(name.into()))
    }

    /// [`MeshKey::Visual`].
    pub fn visual(&self, name: &str) -> MeshHandle {
        self.mesh(MeshKey::Visual(name.into()))
    }

    /// A mesh made by the caller, ready at once and never evicted. Handing over another mesh
    /// under the same name replaces it.
    pub fn add_mesh(&self, name: &str, data: MeshData) -> MeshHandle {
        let uploaded = self.gpu.as_ref().map(|g| upload_mesh(g, &data));
        let mut inner = self.inner.lock();
        let h = mesh_slot(&mut inner, MeshKey::Named(name.into()));
        let ready = finish_mesh_info(&mut inner, &data, uploaded);
        if let Some(slot) = inner.meshes.get_mut(h.0 as usize) {
            let old = slot.bytes;
            slot.state = State::Ready(ready);
            slot.bytes = data.bytes();
            slot.pinned = true;
            slot.version += 1;
            inner.mesh_bytes = inner.mesh_bytes - old + data.bytes();
        }
        drop(inner);
        if h != self.placeholder {
            self.changed();
        }
        h
    }

    /// Lets go of a mesh handed over with [`Assets::add_mesh`]: its GPU copy goes, the handle
    /// draws nothing until a mesh of that name is handed over again.
    pub fn remove_mesh(&self, name: &str) {
        {
            let mut guard = self.inner.lock();
            let inner = &mut *guard;
            let Some(&id) = inner.mesh_ids.get(&MeshKey::Named(name.into())) else { return };
            if id == self.placeholder.0 || id == self.skybox.0 {
                return;
            }
            let Some(slot) = inner.meshes.get_mut(id as usize) else { return };
            inner.mesh_bytes = inner.mesh_bytes.saturating_sub(slot.bytes);
            slot.bytes = 0;
            slot.state = State::Failed("removed".into());
            slot.pinned = false;
            slot.version += 1;
        }
        self.changed();
    }

    /// The handle of a texture by its game name; nothing loads until it is drawn.
    pub fn texture(&self, name: &str) -> TextureHandle {
        let mut inner = self.inner.lock();
        texture_slot(&mut inner, texture_key(name))
    }

    /// A texture made by the caller, ready at once and never evicted.
    pub fn add_texture(&self, name: &str, data: &TextureData) -> Result<TextureHandle> {
        let gpu = match &self.gpu {
            Some(g) => Some(upload_texture(g, data, name)?),
            None => {
                data.check()?;
                None
            }
        };
        let mut inner = self.inner.lock();
        let h = texture_slot(&mut inner, texture_key(name));
        let inner_ref = &mut *inner;
        if let Some(slot) = inner_ref.textures.get_mut(h.0 as usize) {
            inner_ref.texture_bytes = inner_ref.texture_bytes - slot.bytes + data.bytes();
            slot.state = State::Ready(gpu);
            slot.bytes = data.bytes();
            slot.pinned = true;
            slot.version += 1;
        }
        drop(inner);
        self.changed();
        Ok(h)
    }

    /// The handle of a material; the same description gives the same handle.
    pub fn material(&self, desc: MaterialDesc) -> MaterialHandle {
        let mut inner = self.inner.lock();
        material_slot(&mut inner, desc)
    }

    #[must_use]
    pub fn mesh_state(&self, h: MeshHandle) -> AssetState {
        self.inner.lock().meshes.get(h.0 as usize).map_or(AssetState::Missing, |s| s.state.public())
    }

    #[must_use]
    pub fn texture_state(&self, h: TextureHandle) -> AssetState {
        self.inner.lock().textures.get(h.0 as usize).map_or(AssetState::Missing, |s| s.state.public())
    }

    /// The bounds of a loaded mesh.
    #[must_use]
    pub fn mesh_bounds(&self, h: MeshHandle) -> Option<Aabb> {
        match &self.inner.lock().meshes.get(h.0 as usize)?.state {
            State::Ready(r) => Some(r.info.bounds),
            _ => None,
        }
    }

    #[must_use]
    pub fn stats(&self) -> AssetStats {
        let inner = self.inner.lock();
        let failed = inner.meshes.iter().filter(|s| matches!(s.state, State::Failed(_))).count()
            + inner.textures.iter().filter(|s| matches!(s.state, State::Failed(_))).count();
        AssetStats {
            meshes: inner.meshes.len(),
            textures: inner.textures.len(),
            materials: inner.materials.len(),
            mesh_bytes: inner.mesh_bytes,
            texture_bytes: inner.texture_bytes,
            loading: inner.loading,
            failed,
        }
    }

    /// Loads everything `list` draws - its meshes, then their textures - and waits for it, up
    /// to `timeout`. For a thumbnail or a preview picture, which must not show placeholders.
    /// Blocks: never call it on the UI thread. `true` when nothing is left loading.
    pub fn prepare(&self, list: &DrawList, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let tick = self.next_tick();
        let mut meshes: Vec<MeshHandle> = list.instances.iter().map(|i| i.mesh).collect();
        meshes.sort_unstable();
        meshes.dedup();
        let overrides: Vec<MaterialHandle> = list.instances.iter().filter_map(|i| i.material).collect();
        let mut inner = self.inner.lock();
        loop {
            let mut busy = false;
            let mut mats: Vec<MaterialHandle> = overrides.clone();
            for &h in &meshes {
                match self.touch_mesh(&mut inner, h, tick) {
                    Some(info) => mats.extend(info.submeshes.iter().map(|s| s.material)),
                    None => {
                        busy |=
                            matches!(inner.meshes.get(h.0 as usize).map(|s| &s.state), Some(State::Loading))
                    }
                }
            }
            for m in mats {
                busy |= self.touch_material(&mut inner, m, tick);
            }
            if !busy {
                return true;
            }
            if self.loaded.wait_until(&mut inner, deadline).timed_out() {
                return false;
            }
        }
    }

    /// The packet builder's look at meshes: their info when loaded, a load started otherwise,
    /// and whether one that is not there is still to come.
    pub(crate) fn resolve_meshes(
        &self,
        handles: &[MeshHandle],
        tick: u64,
    ) -> Vec<(Option<Arc<MeshInfo>>, bool)> {
        let mut inner = self.inner.lock();
        handles
            .iter()
            .map(|h| {
                let info = self.touch_mesh(&mut inner, *h, tick);
                let pending = info.is_none()
                    && matches!(inner.meshes.get(h.0 as usize).map(|s| &s.state), Some(State::Loading));
                (info, pending)
            })
            .collect()
    }

    /// The packet builder's look at materials: what draws them, and their textures asked for.
    pub(crate) fn resolve_materials(&self, handles: &[MaterialHandle], tick: u64) -> Vec<MaterialInfo> {
        let mut inner = self.inner.lock();
        handles
            .iter()
            .map(|h| {
                self.touch_material(&mut inner, *h, tick);
                inner
                    .materials
                    .get(h.0 as usize)
                    .map_or(MaterialInfo { kind: EditorMaterial::Opaque, two_sided: false }, |m| {
                        MaterialInfo { kind: m.desc.kind, two_sided: m.desc.two_sided }
                    })
            })
            .collect()
    }

    /// The recorder's look at meshes: the GPU copies, `None` for one gone since the packet was
    /// built (it draws the placeholder).
    pub(crate) fn gpu_meshes(&self, handles: &[MeshHandle], out: &mut Vec<Option<Arc<GpuMesh>>>) {
        let inner = self.inner.lock();
        out.clear();
        out.extend(handles.iter().map(|h| match inner.meshes.get(h.0 as usize).map(|s| &s.state) {
            Some(State::Ready(r)) => r.gpu.clone(),
            _ => None,
        }));
    }

    /// The recorder's look at materials: a bind group each, built again when one of its
    /// textures changed. `true` when a texture is still to come.
    pub(crate) fn bind_groups(
        &self,
        handles: &[MaterialHandle],
        out: &mut Vec<Arc<wgpu::BindGroup>>,
    ) -> bool {
        out.clear();
        let Some(gpu) = &self.gpu else { return false };
        let tick = self.tick.load(Ordering::Acquire);
        let mut inner = self.inner.lock();
        let mut pending = false;
        for h in handles {
            pending |= self.touch_material(&mut inner, *h, tick);
            let id = if (h.0 as usize) < inner.materials.len() { h.0 } else { self.default_material.0 };
            if let Some(bg) = material_bind_group(gpu, &mut inner, id) {
                out.push(bg);
            }
        }
        pending
    }

    /// The bind group of the placeholder's grey: what a draw whose material went missing uses.
    pub(crate) fn fallback_bind_group(&self) -> Option<Arc<wgpu::BindGroup>> {
        let gpu = self.gpu.as_ref()?;
        let mut inner = self.inner.lock();
        material_bind_group(gpu, &mut inner, self.default_material.0)
    }

    fn touch_mesh(&self, inner: &mut Inner, h: MeshHandle, tick: u64) -> Option<Arc<MeshInfo>> {
        let slot = inner.meshes.get_mut(h.0 as usize)?;
        slot.last_used = tick;
        if let State::Ready(r) = &slot.state {
            return Some(r.info.clone());
        }
        if matches!(slot.state, State::Missing) {
            match self.start_load() {
                Some(epoch) => {
                    slot.state = State::Loading;
                    let key = slot.key.clone();
                    inner.loading += 1;
                    self.spawn_mesh(h.0, key, epoch);
                }
                None => slot.state = State::Failed("nothing loads models here".into()),
            }
        }
        None
    }

    /// Asks for a material's textures; `true` while one is loading.
    fn touch_material(&self, inner: &mut Inner, h: MaterialHandle, tick: u64) -> bool {
        let Some(m) = inner.materials.get(h.0 as usize) else { return false };
        let (base, detail) = (m.base, m.detail);
        let mut busy = false;
        for t in [base, detail].into_iter().flatten() {
            busy |= self.touch_texture(inner, t, tick);
        }
        busy
    }

    fn touch_texture(&self, inner: &mut Inner, h: TextureHandle, tick: u64) -> bool {
        let Some(slot) = inner.textures.get_mut(h.0 as usize) else { return false };
        slot.last_used = tick;
        if matches!(slot.state, State::Loading) {
            return true;
        }
        if !matches!(slot.state, State::Missing) {
            return false;
        }
        slot.version += 1;
        match self.start_load() {
            Some(epoch) => {
                slot.state = State::Loading;
                let key = slot.key.clone();
                inner.loading += 1;
                self.spawn_texture(h.0, key, epoch);
                true
            }
            None => {
                slot.state = State::Failed("nothing loads textures here".into());
                false
            }
        }
    }

    /// The epoch a load belongs to, when loads can run at all.
    fn start_load(&self) -> Option<u64> {
        (self.gpu.is_some() && self.exec.is_some() && self.loader.read().is_some())
            .then(|| self.epoch.load(Ordering::Acquire))
    }

    fn spawn_mesh(&self, id: u32, key: MeshKey, epoch: u64) {
        let (Some(me), Some(exec)) = (self.me.upgrade(), self.exec.clone()) else { return };
        exec.clone().spawn_io(move || {
            let loader = me.loader.read().clone();
            let what = match &key {
                MeshKey::Object(n) => AssetRef::Object(n),
                MeshKey::Visual(n) => AssetRef::Visual(n),
                MeshKey::Named(n) => AssetRef::Object(n),
            };
            let got = match loader {
                Some(l) => l.load(what),
                None => Err("nothing loads models here".to_owned()),
            };
            match got {
                Ok(bytes) => exec.spawn_cpu(move || {
                    let table = me.table();
                    let data = match &key {
                        MeshKey::Visual(_) => gz_fmt_ogf::Visual::read(&bytes.bytes)
                            .map_err(Into::into)
                            .and_then(|v| MeshData::from_visual(&v, &table)),
                        _ => gz_fmt_object::Object::read(&bytes.bytes)
                            .map_err(Into::into)
                            .and_then(|o| MeshData::from_object(&o, &table)),
                    };
                    me.finish_mesh(id, epoch, data.map_err(|e| e.to_string()));
                }),
                Err(e) => me.finish_mesh(id, epoch, Err(e)),
            }
        });
    }

    fn finish_mesh(&self, id: u32, epoch: u64, data: Result<MeshData, String>) {
        let uploaded = match (&data, &self.gpu) {
            (Ok(d), Some(g)) => Some(upload_mesh(g, d)),
            _ => None,
        };
        {
            let mut inner = self.inner.lock();
            if self.epoch.load(Ordering::Acquire) != epoch {
                return;
            }
            let loading = matches!(inner.meshes.get(id as usize).map(|s| &s.state), Some(State::Loading));
            if !loading {
                return;
            }
            inner.loading = inner.loading.saturating_sub(1);
            let state = match &data {
                Ok(d) => State::Ready(finish_mesh_info(&mut inner, d, uploaded)),
                Err(e) => State::Failed(e.as_str().into()),
            };
            let bytes = data.as_ref().map_or(0, MeshData::bytes);
            if let Some(slot) = inner.meshes.get_mut(id as usize) {
                slot.state = state;
                slot.bytes = bytes;
                slot.version += 1;
            }
            inner.mesh_bytes += bytes;
            evict(&mut inner, &self.budgets, self.tick.load(Ordering::Acquire));
        }
        self.changed();
    }

    fn spawn_texture(&self, id: u32, name: SmolStr, epoch: u64) {
        let (Some(me), Some(exec)) = (self.me.upgrade(), self.exec.clone()) else { return };
        exec.clone().spawn_io(move || {
            let loader = me.loader.read().clone();
            let got = match loader {
                Some(l) => l.load(AssetRef::Texture(&name)),
                None => Err("nothing loads textures here".to_owned()),
            };
            match got {
                Ok(bytes) => exec.spawn_cpu(move || {
                    let result = match &me.gpu {
                        Some(g) => TextureData::decode(&bytes.bytes, bytes.ext.as_deref(), g.bc)
                            .and_then(|d| upload_texture(g, &d, &name).map(|t| (t, d.bytes())))
                            .map_err(|e| e.to_string()),
                        None => Err("there is no graphics device".to_owned()),
                    };
                    me.finish_texture(id, epoch, result);
                }),
                Err(e) => me.finish_texture(id, epoch, Err(e)),
            }
        });
    }

    fn finish_texture(&self, id: u32, epoch: u64, result: Result<(Arc<GpuTexture>, usize), String>) {
        {
            let mut inner = self.inner.lock();
            if self.epoch.load(Ordering::Acquire) != epoch {
                return;
            }
            let Some(slot) = inner.textures.get_mut(id as usize) else { return };
            if !matches!(slot.state, State::Loading) {
                return;
            }
            let bytes = result.as_ref().map_or(0, |(_, b)| *b);
            slot.state = match result {
                Ok((t, _)) => State::Ready(Some(t)),
                Err(e) => State::Failed(e.into()),
            };
            slot.bytes = bytes;
            slot.version += 1;
            inner.loading = inner.loading.saturating_sub(1);
            inner.texture_bytes += bytes;
            evict(&mut inner, &self.budgets, self.tick.load(Ordering::Acquire));
        }
        self.changed();
    }
}

fn mesh_slot(inner: &mut Inner, key: MeshKey) -> MeshHandle {
    if let Some(id) = inner.mesh_ids.get(&key) {
        return MeshHandle(*id);
    }
    let id = inner.meshes.len() as u32;
    inner.meshes.push(Slot::new(key.clone()));
    inner.mesh_ids.insert(key, id);
    MeshHandle(id)
}

fn texture_slot(inner: &mut Inner, key: SmolStr) -> TextureHandle {
    if let Some(id) = inner.texture_ids.get(&key) {
        return TextureHandle(*id);
    }
    let id = inner.textures.len() as u32;
    inner.textures.push(Slot::new(key.clone()));
    inner.texture_ids.insert(key, id);
    TextureHandle(id)
}

fn material_slot(inner: &mut Inner, desc: MaterialDesc) -> MaterialHandle {
    let key = desc.key();
    if let Some(id) = inner.material_ids.get(&key) {
        return MaterialHandle(*id);
    }
    let base = desc.base.clone().map(|n| texture_slot(inner, n));
    let detail = desc.detail.clone().map(|n| texture_slot(inner, n));
    let id = inner.materials.len() as u32;
    inner.materials.push(MatSlot { desc, base, detail, ubo: None, bind: None });
    inner.material_ids.insert(key, id);
    MaterialHandle(id)
}

type Uploaded = (wgpu::Buffer, wgpu::Buffer, wgpu::Buffer, Vec<Range<u32>>);

fn finish_mesh_info(inner: &mut Inner, data: &MeshData, uploaded: Option<Uploaded>) -> MeshReady {
    let wires: Vec<Range<u32>> = uploaded.as_ref().map(|u| u.3.clone()).unwrap_or_default();
    let submeshes = data
        .submeshes
        .iter()
        .enumerate()
        .map(|(i, s)| SubInfo {
            indices: s.indices.clone(),
            wire: wires.get(i).cloned().unwrap_or(0..0),
            material: material_slot(inner, s.material.clone()),
        })
        .collect();
    let info = Arc::new(MeshInfo { bounds: data.bounds, submeshes, skinned: data.skinned });
    let gpu = uploaded.map(|(vertices, indices, wire, _)| {
        Arc::new(GpuMesh { vertices, indices, wire, info: info.clone() })
    });
    MeshReady { info, gpu }
}

fn upload_mesh(gpu: &AssetGpu, data: &MeshData) -> Uploaded {
    use wgpu::util::DeviceExt;
    let (wire, ranges) = data.wire_indices();
    let make = |label: &str, contents: &[u8], usage: wgpu::BufferUsages| {
        gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents,
            usage,
        })
    };
    let wire_bytes: &[u8] = if wire.is_empty() { &[0u8; 8] } else { bytemuck::cast_slice(&wire) };
    (
        make("gz-render mesh vertices", bytemuck::cast_slice(&data.vertices), wgpu::BufferUsages::VERTEX),
        make("gz-render mesh indices", bytemuck::cast_slice(&data.indices), wgpu::BufferUsages::INDEX),
        make("gz-render mesh edges", wire_bytes, wgpu::BufferUsages::INDEX),
        ranges,
    )
}

fn upload_texture(gpu: &AssetGpu, data: &TextureData, label: &str) -> Result<Arc<GpuTexture>> {
    upload_texture_to(&gpu.device, &gpu.queue, gpu.bc, gpu.max_side, data, label)
}

/// Creates a texture and queues its levels.
pub(crate) fn upload_texture_to(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    bc: bool,
    max_side: u32,
    data: &TextureData,
    label: &str,
) -> Result<Arc<GpuTexture>> {
    data.check()?;
    if data.format.is_block() && !bc {
        return Err(invalid("the graphics device does not sample block-compressed textures"));
    }
    let layers = data.layers.max(1);
    let mips = data.mips().max(1);
    // a texture larger than the device takes loses its top levels
    let mut skip = 0u32;
    while skip + 1 < mips {
        let (w, h) = data.level_size(skip);
        if w.max(h) <= max_side {
            break;
        }
        skip += 1;
    }
    let (w, h) = data.level_size(skip);
    if w.max(h) > max_side || (data.format.is_block() && (w % 4 != 0 || h % 4 != 0)) {
        return Err(invalid(format!(
            "a texture of {} x {} is larger than the graphics device takes",
            data.width, data.height
        )));
    }
    let format = data.format.wgpu();
    let levels = mips - skip;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: layers },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    for layer in 0..layers {
        let chain = data.levels.iter().skip((layer * mips + skip) as usize).take(levels as usize);
        for (mip, level) in chain.enumerate() {
            let (lw, lh) = data.level_size(mip as u32 + skip);
            let (row, rows) = level_layout(data.format, lw, lh);
            let extent = if data.format.is_block() {
                wgpu::Extent3d {
                    width: lw.div_ceil(4) * 4,
                    height: lh.div_ceil(4) * 4,
                    depth_or_array_layers: 1,
                }
            } else {
                wgpu::Extent3d { width: lw, height: lh, depth_or_array_layers: 1 }
            };
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: mip as u32,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                    aspect: wgpu::TextureAspect::All,
                },
                level,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(rows),
                },
                extent,
            );
        }
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2),
        base_array_layer: 0,
        array_layer_count: Some(1),
        ..wgpu::TextureViewDescriptor::default()
    });
    let cube = data.is_cube().then(|| {
        texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::Cube),
            array_layer_count: Some(6),
            ..wgpu::TextureViewDescriptor::default()
        })
    });
    Ok(Arc::new(GpuTexture { _texture: texture, view, cube }))
}

/// The uniform block of a material.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MaterialGpu {
    color: [f32; 4],
    /// Alpha reference, detail scale, 1 with a detail texture.
    params: [f32; 4],
}

fn material_bind_group(gpu: &AssetGpu, inner: &mut Inner, id: u32) -> Option<Arc<wgpu::BindGroup>> {
    use wgpu::util::DeviceExt;
    let m = inner.materials.get(id as usize)?;
    let view_of = |h: Option<TextureHandle>| -> (Arc<GpuTexture>, u64) {
        let Some(h) = h else { return (gpu.white.clone(), 0) };
        match inner.textures.get(h.0 as usize) {
            Some(Slot { state: State::Ready(Some(t)), version, .. }) => (t.clone(), *version),
            Some(s) => (gpu.checker.clone(), s.version),
            None => (gpu.checker.clone(), 0),
        }
    };
    let (base, vb) = view_of(m.base);
    let (detail, vd) = view_of(m.detail);
    let stamp = (vb, vd);
    if let Some((bg, s)) = &m.bind
        && *s == stamp
    {
        return Some(bg.clone());
    }
    let desc = m.desc.clone();
    let m = inner.materials.get_mut(id as usize)?;
    let has_detail = if desc.detail.is_some() { 1.0 } else { 0.0 };
    // which of the textures are cube maps is known once they are loaded: the block follows
    let cubes = u32::from(base.cube.is_some()) | (u32::from(detail.cube.is_some()) << 1);
    let block = MaterialGpu {
        color: desc.color.to_f32(),
        params: [desc.alpha_ref, desc.detail_scale, has_detail, cubes as f32],
    };
    let ubo = match &m.ubo {
        Some(u) => {
            gpu.queue.write_buffer(u, 0, bytemuck::bytes_of(&block));
            u
        }
        None => m.ubo.insert(gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gz-render material"),
            contents: bytemuck::bytes_of(&block),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })),
    };
    let base_cube = base.cube.as_ref().or(gpu.white_cube.cube.as_ref()).unwrap_or(&gpu.white_cube.view);
    let detail_cube = detail.cube.as_ref().or(gpu.white_cube.cube.as_ref()).unwrap_or(&gpu.white_cube.view);
    let bg = Arc::new(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gz-render material"),
        layout: &gpu.material_layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: ubo.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&base.view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&detail.view) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&gpu.sampler) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(base_cube) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(detail_cube) },
        ],
    }));
    m.bind = Some((bg.clone(), stamp));
    Some(bg)
}

/// Drops the least recently used until the budgets hold, never what the current or the last
/// packet drew.
fn evict(inner: &mut Inner, budgets: &Budgets, tick: u64) {
    let keep_from = tick.saturating_sub(1);
    if inner.mesh_bytes > budgets.mesh_bytes {
        let mut order: Vec<(u64, usize)> = inner
            .meshes
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.pinned && matches!(s.state, State::Ready(_)) && s.last_used < keep_from)
            .map(|(i, s)| (s.last_used, i))
            .collect();
        order.sort_unstable();
        for (_, i) in order {
            if inner.mesh_bytes <= budgets.mesh_bytes {
                break;
            }
            if let Some(s) = inner.meshes.get_mut(i) {
                inner.mesh_bytes -= s.bytes;
                s.bytes = 0;
                s.state = State::Missing;
                s.version += 1;
            }
        }
    }
    if inner.texture_bytes > budgets.texture_bytes {
        let mut order: Vec<(u64, usize)> = inner
            .textures
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.pinned && matches!(s.state, State::Ready(_)) && s.last_used < keep_from)
            .map(|(i, s)| (s.last_used, i))
            .collect();
        order.sort_unstable();
        let mut gone = Vec::new();
        for (_, i) in order {
            if inner.texture_bytes <= budgets.texture_bytes {
                break;
            }
            if let Some(s) = inner.textures.get_mut(i) {
                inner.texture_bytes -= s.bytes;
                s.bytes = 0;
                s.state = State::Missing;
                s.version += 1;
                gone.push(TextureHandle(i as u32));
            }
        }
        // a bind group keeps its textures alive: let go of the ones that held these
        for m in &mut inner.materials {
            if m.base.is_some_and(|b| gone.contains(&b)) || m.detail.is_some_and(|d| gone.contains(&d)) {
                m.bind = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::material::MaterialDesc;

    #[test]
    fn handles_intern_by_content() {
        let a = Assets::headless(MaterialTable::default());
        assert_eq!(a.object("Trees/Tree_01"), a.object("trees\\tree_01.object"));
        assert_ne!(a.object("trees\\tree_01"), a.visual("trees\\tree_01"));
        assert_eq!(a.texture("A/B.dds"), a.texture("a\\b"));
        let m1 = a.material(MaterialDesc::textured(EditorMaterial::Opaque, "x"));
        let m2 = a.material(MaterialDesc::textured(EditorMaterial::Opaque, "x"));
        let m3 = a.material(MaterialDesc::textured(EditorMaterial::AlphaTest, "x"));
        assert_eq!(m1, m2);
        assert_ne!(m1, m3);
        // the placeholder's grey is made with the table: two distinct ones and it
        assert_eq!(a.stats().materials, 3 + 1);
    }

    #[test]
    fn headless_meshes_are_ready_or_fail() {
        let a = Assets::headless(MaterialTable::default());
        let cube =
            a.add_mesh("cube", MeshData::cube(MaterialDesc::textured(EditorMaterial::Opaque, "crate")));
        assert_eq!(a.mesh_state(cube), AssetState::Ready);
        assert_eq!(a.mesh_bounds(cube), Some(Aabb::UNIT));
        let far = a.object("nowhere");
        let got = a.resolve_meshes(&[cube, far], a.next_tick());
        assert!(got[0].0.is_some() && got[1].0.is_none());
        // failed, not coming
        assert!(!got[1].1);
        assert!(matches!(a.mesh_state(far), AssetState::Failed(_)));
        let info = got[0].0.clone().unwrap();
        let mats = a.resolve_materials(&[info.submeshes[0].material], 1);
        assert_eq!(mats[0].kind, EditorMaterial::Opaque);
        // nothing can load the texture: it fails rather than waits
        assert!(a.prepare(&DrawList::default(), Duration::from_millis(1)));
    }
}
