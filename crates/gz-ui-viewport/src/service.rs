//! [`Viewport`]: the view state the pane, the MCP handlers and the scene's keys share, the
//! renderer, and the scene pass - a packet built on the CPU pool whenever something it draws
//! changed, and never otherwise.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use glam::Vec3;
use gz_base::Rev;
use gz_editor::{CmdError, Editor, SmolStr};
use gz_render::{Color, FramePacket, Image, Renderer, View, ViewFlags, ViewMode};
use gz_scene::{ObjClass, SceneService};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::camera::Nav;
use crate::draw::{self, DrawOptions};
use crate::gizmo::{Anchor, GizmoType};
use crate::lighting::{self, GameLight, GameSettings, Weather};
use crate::loader::SceneLoader;
use crate::session::{self, Saved};

/// What a click in the viewport does (`ETAction`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Action {
    #[default]
    Select,
    /// `etaAdd`: a click adds what the Tools panel has picked.
    Place,
}

/// How surfaces are filled (`dwFillMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fill {
    #[default]
    Solid,
    Wireframe,
    /// Drawn as the wireframe: the renderer has no point mode.
    Point,
}

impl Fill {
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Fill::Solid => "solid",
            Fill::Wireframe => "wireframe",
            Fill::Point => "point",
        }
    }
}

/// The render flags `view_mode` reads and sets (`psDeviceFlags`), and the spawn helpers.
#[derive(Clone, Debug, PartialEq)]
pub struct ViewOpts {
    pub fill: Fill,
    pub flat: bool,
    /// The studio light; off draws the textures as they are.
    pub lighting: bool,
    pub textures: bool,
    pub edged_faces: bool,
    pub filter_linear: bool,
    pub fog: bool,
    pub environment: bool,
    pub grid: bool,
    pub safe_rect: bool,
    pub stats: bool,
    /// `g_SpawnHelpers`.
    pub helpers: bool,
}

impl Default for ViewOpts {
    fn default() -> Self {
        Self {
            fill: Fill::Solid,
            flat: false,
            lighting: true,
            textures: true,
            edged_faces: false,
            filter_linear: true,
            fog: false,
            environment: false,
            grid: true,
            safe_rect: false,
            stats: false,
            helpers: true,
        }
    }
}

/// The transform tool and its snaps (`LTools->GetAction`, the gizmo's type and steps).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tool {
    pub action: Action,
    pub gizmo: GizmoType,
    /// The grid, scale and angle snaps are on.
    pub grid_snap: bool,
    pub scale_snap: bool,
    pub angle_snap: bool,
}

/// `Tools->GetSettings`: object snap, vertex snap, normal alignment, picking any class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Settings {
    pub object_snap: bool,
    pub vertex_snap: bool,
    pub normal_align: bool,
    pub pick_any_class: bool,
}

impl Default for Settings {
    /// `CToolCustom`'s defaults.
    fn default() -> Self {
        Self { object_snap: true, vertex_snap: true, normal_align: true, pick_any_class: true }
    }
}

/// View > Game Lighting (`EGameLighting`).
#[derive(Clone, Debug, PartialEq)]
pub struct GameLighting {
    pub enabled: bool,
    /// The weather asked for; empty is the default for the hour.
    pub weather: String,
    pub hour: f32,
    /// 0: the game's tonemap; else this factor.
    pub exposure: f32,
    pub auto_exposure: f32,
    pub scene_lum: f32,
    pub indoor_hemi: f32,
    pub at: Weather,
    pub weather_ok: bool,
    pub error: String,
    pub settings: GameSettings,
    /// The lights the last frame drew, and how many were in view before the cut.
    pub drawn: Vec<GameLight>,
    pub in_view: usize,
    pub roof: Option<Arc<gz_render::RoofMap>>,
    pub roof_bake_ms: u32,
    /// Rebake Roof Map was asked.
    pub rebake: bool,
    /// The weathers of the linked game, read off the UI thread.
    pub weathers: Vec<String>,
    /// A read of the weather is under way.
    pub loading: bool,
}

impl Default for GameLighting {
    fn default() -> Self {
        Self {
            enabled: false,
            weather: String::new(),
            hour: 22.0,
            exposure: 0.0,
            auto_exposure: 0.0,
            scene_lum: 0.0,
            indoor_hemi: 0.15,
            at: Weather { sun_dir: Vec3::new(0.0, -1.0, 0.0), ..Weather::default() },
            weather_ok: false,
            error: String::new(),
            settings: GameSettings::default(),
            drawn: Vec::new(),
            in_view: 0,
            roof: None,
            roof_bake_ms: 0,
            rebake: false,
            weathers: Vec::new(),
            loading: false,
        }
    }
}

impl GameLighting {
    #[must_use]
    pub fn exposure_now(&self) -> f32 {
        self.settings.exposure(self.exposure, self.auto_exposure)
    }

    /// What the frame lights with: on, with a weather that read.
    #[must_use]
    pub fn active(&self) -> bool {
        self.enabled && self.weather_ok
    }
}

/// The preferences the view follows, as last applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Prefs {
    pub near: f32,
    pub far: f32,
    pub fov: f32,
    pub cam_sens_move: f32,
    pub cam_sens_rot: f32,
    pub fly_speed: f32,
    pub fly_alt: f32,
    pub tools_sens_move: f32,
    pub tools_sens_rot: f32,
    pub tools_sens_scale: f32,
    pub grid_size: f32,
    pub grid_count: u32,
    pub snap_grid: f32,
    pub snap_angle: f32,
    pub snap_scale: f32,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            near: 0.1,
            far: 1500.0,
            fov: 60.0,
            cam_sens_move: 0.6,
            cam_sens_rot: 0.6,
            fly_speed: 5.0,
            fly_alt: 1.8,
            tools_sens_move: 0.3,
            tools_sens_rot: 0.3,
            tools_sens_scale: 0.3,
            grid_size: 1.0,
            grid_count: 100,
            snap_grid: 0.1,
            snap_angle: 5.625,
            snap_scale: 0.1,
        }
    }
}

impl Prefs {
    fn read(editor: &Editor) -> Self {
        let s = gz_app::EditorSettings::of(editor);
        let d = Self::default();
        let f = |k: &str, def: f32| s.pref(k).map_or(def, |v| v as f32);
        Self {
            near: f("view_np", d.near),
            far: f("view_fp", d.far),
            fov: f("view_fov", d.fov),
            cam_sens_move: f("cam_sens_move", d.cam_sens_move),
            cam_sens_rot: f("cam_sens_rot", d.cam_sens_rot),
            fly_speed: f("cam_fly_speed", d.fly_speed),
            fly_alt: f("cam_fly_alt", d.fly_alt),
            tools_sens_move: f("tools_sens_move", d.tools_sens_move),
            tools_sens_rot: f("tools_sens_rot", d.tools_sens_rot),
            tools_sens_scale: f("tools_sens_scale", d.tools_sens_scale),
            grid_size: f("grid_cell_size", d.grid_size),
            grid_count: s.pref("grid_cell_count").map_or(d.grid_count, |v| v as u32),
            snap_grid: f("snap_grid_step", d.snap_grid),
            snap_angle: f("snap_angle_step", d.snap_angle),
            snap_scale: f("snap_scale_step", d.snap_scale),
        }
    }

    /// `m_MouseSR`: the tools' rotation per pixel.
    #[must_use]
    pub fn tools_sr(&self) -> f32 {
        0.02 * self.tools_sens_rot * self.tools_sens_rot
    }
}

/// Everything the view is: one lock, never held across an await.
#[derive(Clone, Debug)]
pub struct ViewState {
    pub nav: Nav,
    pub view: ViewOpts,
    pub tool: Tool,
    pub settings: Settings,
    pub game: GameLighting,
    pub prefs: Prefs,
    /// The render size in pixels the pane last laid out (1280 x 720 before it ever did).
    pub size: [u32; 2],
    /// The gizmo's anchor of the current class's selection, as the last scene pass found it.
    pub anchor: Option<Anchor>,
    /// What Place adds, per class (the Tools panel's pick).
    pub place: FxHashMap<ObjClass, SmolStr>,
    /// A short line the pane shows for a moment (the camera's speed).
    pub status: Option<(String, Instant)>,
    /// What is behind the scene: the theme's canvas colour, as the pane last drew it.
    pub background: Color,
    scene_key: Option<String>,
    cameras: FxHashMap<String, Saved>,
    cameras_for: Option<PathBuf>,
    cameras_read: bool,
    saved_pose: Option<Saved>,
    pose_changed: Option<Instant>,
    pacing: Option<(u64, Instant)>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            nav: Nav::default(),
            view: ViewOpts::default(),
            tool: Tool::default(),
            settings: Settings::default(),
            game: GameLighting::default(),
            prefs: Prefs::default(),
            size: [1280, 720],
            anchor: None,
            place: FxHashMap::default(),
            status: None,
            // the kit's token `canvas` (151419) until the pane draws with the theme's
            background: Color::rgb(0x15, 0x14, 0x19),
            scene_key: None,
            cameras: FxHashMap::default(),
            cameras_for: None,
            cameras_read: false,
            saved_pose: None,
            pose_changed: None,
            pacing: None,
        }
    }
}

impl ViewState {
    /// The steps of the grid, scale and angle snaps (the preferences), and whether each is on.
    #[must_use]
    pub fn step(&self, kind: GizmoType) -> f32 {
        match kind {
            GizmoType::Move => self.prefs.snap_grid,
            GizmoType::Scale => self.prefs.snap_scale,
            GizmoType::Rotate => self.prefs.snap_angle,
            GizmoType::None => 0.0,
        }
    }

    #[must_use]
    pub fn snap_on(&self, kind: GizmoType) -> bool {
        match kind {
            GizmoType::Move => self.tool.grid_snap,
            GizmoType::Scale => self.tool.scale_snap,
            GizmoType::Rotate => self.tool.angle_snap,
            GizmoType::None => false,
        }
    }

    /// `aspect` as the camera's maths use it: height over width (`fASPECT`).
    #[must_use]
    pub fn aspect(&self) -> f32 {
        if self.size[0] == 0 { 1.0 } else { self.size[1] as f32 / self.size[0] as f32 }
    }
}

/// What decides whether a new packet is needed.
#[derive(Clone, Debug, PartialEq)]
struct BuildKey {
    scene: Rev,
    library: Rev,
    view: u64,
    assets: u64,
}

#[derive(Default)]
struct BuildCtl {
    in_flight: bool,
    again: bool,
    last: Option<BuildKey>,
    packet: Option<Arc<FramePacket>>,
    serial: u64,
}

/// The game's data Game Lighting reads off the UI thread, per link revision.
#[derive(Clone, Default)]
struct GameData {
    rev: Option<Rev>,
    configs: Option<Arc<gz_ltx::Ltx>>,
    anims: Option<Arc<gz_fmt_xr::LightAnims>>,
}

/// The viewport of this editor: an `Editor` service.
pub struct Viewport {
    state: Mutex<ViewState>,
    /// Moves with every change of the view state.
    rev: AtomicU64,
    renderer: Mutex<Option<Arc<Renderer>>>,
    renderer_failed: Mutex<bool>,
    build: Mutex<BuildCtl>,
    loader_key: Mutex<Option<(Option<PathBuf>, Rev)>>,
    game_data: Mutex<GameData>,
    /// The pane's own frame rate and the renderer's, for the stats pill.
    pub(crate) rates: Mutex<Rates>,
}

/// Frames per second, measured over about half a second.
#[derive(Clone, Debug, Default)]
pub(crate) struct Rates {
    pub ui_fps: f32,
    pub render_fps: f32,
    since: Option<Instant>,
    ui_frames: u32,
    render_frame: u64,
}

impl Rates {
    pub(crate) fn tick(&mut self, render_frame: u64) {
        let now = Instant::now();
        self.ui_frames += 1;
        match self.since {
            None => {
                self.since = Some(now);
                self.render_frame = render_frame;
            }
            Some(t) => {
                let dt = now.duration_since(t).as_secs_f32();
                if dt >= 0.5 {
                    self.ui_fps = self.ui_frames as f32 / dt;
                    self.render_fps = render_frame.saturating_sub(self.render_frame) as f32 / dt;
                    self.since = Some(now);
                    self.ui_frames = 0;
                    self.render_frame = render_frame;
                }
            }
        }
    }
}

impl Viewport {
    /// The viewport of this editor, made on first use.
    pub fn of(editor: &Editor) -> Arc<Viewport> {
        editor.services.get_or_init(|| Viewport {
            state: Mutex::new(ViewState::default()),
            rev: AtomicU64::new(1),
            renderer: Mutex::new(None),
            renderer_failed: Mutex::new(false),
            build: Mutex::new(BuildCtl::default()),
            loader_key: Mutex::new(None),
            game_data: Mutex::new(GameData::default()),
            rates: Mutex::new(Rates::default()),
        })
    }

    /// Changes the view state: the next scene pass draws it.
    pub fn with<R>(&self, f: impl FnOnce(&mut ViewState) -> R) -> R {
        let r = f(&mut self.state.lock());
        self.rev.fetch_add(1, Ordering::AcqRel);
        r
    }

    /// Reads the view state.
    pub fn read<R>(&self, f: impl FnOnce(&ViewState) -> R) -> R {
        f(&self.state.lock())
    }

    #[must_use]
    pub fn rev(&self) -> u64 {
        self.rev.load(Ordering::Acquire)
    }

    /// The renderer, once a window's device made one.
    #[must_use]
    pub fn renderer(&self) -> Option<Arc<Renderer>> {
        self.renderer.lock().clone()
    }

    /// The renderer on the device the window draws with, made the first time.
    pub fn ensure_renderer(&self, editor: &Editor, gpu: &egui_wgpu::RenderState) -> Option<Arc<Renderer>> {
        let mut r = self.renderer.lock();
        if r.is_none() && !*self.renderer_failed.lock() {
            match Renderer::new(gpu.device.clone(), gpu.queue.clone(), editor.exec.clone()) {
                Ok(made) => *r = Some(Arc::new(made)),
                Err(e) => {
                    *self.renderer_failed.lock() = true;
                    editor.log.error(format!("the viewport's renderer did not start: {e}"));
                }
            }
        }
        r.clone()
    }

    /// The renderer, from the window's device when the pane never drew (a picture asked over
    /// MCP while another tab is up).
    pub fn renderer_any(&self, editor: &Editor) -> Option<Arc<Renderer>> {
        if let Some(r) = self.renderer() {
            return Some(r);
        }
        let gpu = editor.services.get::<gz_app::WindowLink>()?.gpu()?;
        self.ensure_renderer(editor, &gpu)
    }

    /// Follows the editor: the preferences, the asset loader of the open project and its game,
    /// the camera of the scene that is open. Cheap; runs every frame and every pass.
    pub fn sync(&self, editor: &Arc<Editor>) {
        let prefs = Prefs::read(editor);
        let changed = self.read(|s| s.prefs != prefs);
        if changed {
            self.with(|s| {
                let fly_changed = s.prefs.fly_speed != prefs.fly_speed;
                s.prefs = prefs;
                s.nav.near = prefs.near;
                s.nav.far = prefs.far;
                s.nav.fov = prefs.fov;
                s.nav.set_sensitivity(prefs.cam_sens_move, prefs.cam_sens_rot);
                s.nav.fly_alt = prefs.fly_alt;
                if fly_changed {
                    s.nav.fly_speed = prefs.fly_speed;
                }
            });
        }
        self.sync_loader(editor);
        self.sync_scene_camera(editor);
    }

    fn sync_loader(&self, editor: &Arc<Editor>) {
        let Some(renderer) = self.renderer() else { return };
        let project = editor.project_root();
        let game_rev = gz_game::GameLink::of(editor).map(|g| g.rev()).unwrap_or(Rev::ZERO);
        let key = (project.clone(), game_rev);
        {
            let mut k = self.loader_key.lock();
            if k.as_ref() == Some(&key) {
                return;
            }
            *k = Some(key);
        }
        let ed = editor.clone();
        let editor_root = editor.paths.root.clone();
        editor.exec.spawn(async move {
            let game = match gz_game::GameLink::of(&ed) {
                Ok(g) => g.files().await.ok(),
                Err(_) => None,
            };
            let loader = SceneLoader::new(project, editor_root, game);
            renderer.assets().set_loader(Some(Arc::new(loader)));
        });
    }

    fn sync_scene_camera(&self, editor: &Arc<Editor>) {
        let Some(root) = editor.project_root() else { return };
        // the project's cameras are read once, off the UI thread
        if self.read(|s| s.cameras_for.as_ref() != Some(&root)) {
            self.state_quiet(|s| {
                s.cameras_for = Some(root.clone());
                s.cameras.clear();
                s.cameras_read = false;
                s.scene_key = None;
            });
            let vp = Viewport::of(editor);
            let file = root.join("temp").join(session::FILE);
            let r = root.clone();
            editor.exec.spawn_io(move || {
                let text = std::fs::read(&file)
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_default();
                let cams = session::parse(&text);
                vp.with(|s| {
                    if s.cameras_for.as_ref() == Some(&r) {
                        for (k, v) in cams {
                            s.cameras.entry(k).or_insert(v);
                        }
                        s.cameras_read = true;
                        s.scene_key = None;
                    }
                });
            });
            return;
        }
        if !self.read(|s| s.cameras_read) {
            return;
        }
        let Some(svc) = SceneService::of_opt(editor) else { return };
        let snap = svc.store.snapshot();
        let key = snap.meta.file.as_ref().map(|f| session::scene_key(&root, f));
        let (current, pose) =
            self.read(|s| (s.scene_key.clone(), Saved { hpb: s.nav.hpb, position: s.nav.position }));
        if key != current {
            let meta = (snap.meta.camera_hpb, snap.meta.camera_pos);
            self.with(|s| {
                if let Some(old) = &current {
                    s.cameras.insert(old.clone(), pose);
                }
                if let Some(k) = &key {
                    let saved = s.cameras.get(k).copied().or_else(|| {
                        (meta.0 != Vec3::ZERO || meta.1 != Vec3::ZERO)
                            .then_some(Saved { hpb: meta.0, position: meta.1 })
                    });
                    if let Some(c) = saved {
                        s.nav.set(c.hpb, c.position);
                        s.nav.target = c.position + s.nav.direction() * 10.0;
                        s.nav.reset_state();
                    }
                }
                s.scene_key = key.clone();
                s.saved_pose = Some(Saved { hpb: s.nav.hpb, position: s.nav.position });
                s.pose_changed = None;
            });
            return;
        }
        // the pose is kept a moment after the camera stops
        let now = Instant::now();
        let write = self.with(|s| {
            let pose = Saved { hpb: s.nav.hpb, position: s.nav.position };
            if s.saved_pose != Some(pose) {
                s.saved_pose = Some(pose);
                s.pose_changed = Some(now);
                return None;
            }
            let due = s.pose_changed.is_some_and(|t| now.duration_since(t) > Duration::from_millis(1500));
            if !due {
                return None;
            }
            s.pose_changed = None;
            let k = s.scene_key.clone()?;
            s.cameras.insert(k, pose);
            Some(session::write(&s.cameras))
        });
        if let Some(text) = write {
            let file = root.join("temp").join(session::FILE);
            let log = editor.log.clone();
            editor.exec.spawn_io(move || {
                if let Some(dir) = file.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                if let Err(e) = gz_editor::fsx::write_atomic(&file, text.as_bytes()) {
                    log.warn(format!("the viewport's camera could not be kept in {}: {e}", file.display()));
                }
            });
        }
    }

    /// Asks for a scene pass when anything it draws changed. Never blocks: the pass is built on
    /// the CPU pool and handed to the render thread.
    pub fn request_build(self: &Arc<Self>, editor: &Arc<Editor>) {
        let Some(renderer) = self.renderer() else { return };
        let key = self.key(editor, &renderer);
        {
            let mut b = self.build.lock();
            let stale = b.packet.as_ref().is_some_and(|p| p.is_stale(renderer.assets()));
            if b.last.as_ref() == Some(&key) && !stale {
                return;
            }
            if b.in_flight {
                b.again = true;
                return;
            }
            b.in_flight = true;
            b.again = false;
        }
        let vp = self.clone();
        let ed = editor.clone();
        editor.exec.spawn(async move {
            let made = vp.make_packet(&ed, &renderer, false).await;
            let again = {
                let mut b = vp.build.lock();
                b.in_flight = false;
                if let Some(p) = &made {
                    b.serial = renderer.submit(p.clone());
                    b.packet = Some(p.clone());
                    b.last = Some(key);
                }
                std::mem::take(&mut b.again)
            };
            ed.exec.wake().ping();
            if again {
                vp.request_build(&ed);
            }
        });
    }

    fn key(&self, editor: &Editor, renderer: &Renderer) -> BuildKey {
        let (scene, library) = match SceneService::of_opt(editor) {
            Some(svc) => (svc.store.rev(), svc.library.rev()),
            None => (Rev::ZERO, Rev::ZERO),
        };
        let stale = self.build.lock().packet.as_ref().is_some_and(|p| p.waiting_on_assets());
        BuildKey {
            scene,
            library,
            view: self.rev(),
            assets: if stale { renderer.assets().generation() } else { 0 },
        }
    }

    /// Waits until no pass is being built: a picture asked over MCP draws the newest state.
    pub async fn settle(&self) {
        for _ in 0..200 {
            if !self.build.lock().in_flight {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Builds a packet of the scene as the view stands now (`preview`: the picture of the scene
    /// alone). Publishes what the pass found: the gizmo's anchor, the lights drawn, the roof map.
    pub(crate) async fn make_packet(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        renderer: &Arc<Renderer>,
        preview: bool,
    ) -> Option<Arc<FramePacket>> {
        self.sync(editor);
        let game_on = self.read(|s| s.game.active());
        let game_data = if game_on { self.game_data(editor).await } else { GameData::default() };
        let scene = SceneService::of_opt(editor);
        let (snap, pick, lib) = match &scene {
            Some(svc) => {
                let (snap, pick) = svc.pick().await;
                (Some(snap), Some(pick), Some(svc.library.clone()))
            }
            None => (None, None, None),
        };
        let st = self.read(Clone::clone);
        let assets = renderer.assets().clone();
        let built = editor
            .exec
            .cpu(move || {
                let cam = st.nav.camera();
                let size = st.size;
                let mut view = View::new(cam, size);
                view.background = st.background;
                view.mode = if st.view.fill != Fill::Solid {
                    ViewMode::Wireframe
                } else if st.game.active() {
                    ViewMode::GameLighting
                } else if st.view.lighting {
                    ViewMode::Lit
                } else {
                    ViewMode::Unlit
                };
                view.flags = ViewFlags::HELPERS | ViewFlags::SELECTION;
                if st.view.grid {
                    view.flags |= ViewFlags::GRID;
                }
                view.grid.cells = st.prefs.grid_count.max(1);
                view.grid.cell_size = st.prefs.grid_size.max(0.01);
                if preview {
                    view = view.as_preview();
                }
                let vp_m = cam.view_proj(size);
                let frustum = cam.frustum(size);
                let mut roof = st.game.roof.clone();
                let mut baked: Option<(u32, usize)> = None;
                let mut gathered: Option<(Vec<GameLight>, usize)> = None;
                let empty_state = gz_scene::SceneState::default();
                let empty_lib = gz_scene::library::Library::new(Vec::new());
                let state = snap.as_ref().map_or(&empty_state, |s| &s.state);
                let library = lib.as_deref().unwrap_or(&empty_lib);
                let game = if st.game.active() {
                    if st.game.rebake || lighting::roof_stale(roof.as_deref(), cam.position) {
                        let t = Instant::now();
                        let revision = roof.as_ref().map_or(1, |r| r.revision + 1);
                        let (map, count) = lighting::bake_roof(state, library, cam.position, revision);
                        baked = Some((t.elapsed().as_millis() as u32, count));
                        roof = Some(Arc::new(map));
                    }
                    let all =
                        lighting::collect(state, game_data.configs.as_deref(), game_data.anims.as_deref());
                    let (drawn, in_view) = lighting::gather(&all, &frustum, cam.position);
                    let exposure = st.game.exposure_now();
                    view.background = lighting::clear_color(&st.game.at, exposure);
                    let env = lighting::environment(
                        &st.game.at,
                        &st.game.settings,
                        exposure,
                        st.game.indoor_hemi,
                        roof.clone(),
                    );
                    let lights = drawn.iter().map(GameLight::render).collect();
                    gathered = Some((drawn, in_view));
                    Some((env, lights))
                } else {
                    None
                };
                let opts = DrawOptions { helpers: st.view.helpers && !preview, game };
                let built = draw::build(state, library, &assets, &opts);
                let packet = match &pick {
                    Some(pick) => {
                        let reach = draw::frustum_reach(&vp_m);
                        let vis = draw::visibility(&built, state, pick, &assets, &frustum, &reach);
                        gz_render::build_packet_with(&built.list, &view, &assets, Some(&vis))
                    }
                    None => gz_render::build_packet(&built.list, &view, &assets),
                };
                let anchor = Anchor::of(&draw::gizmo_points(state, library, state.tools.target));
                (Arc::new(packet), anchor, roof, baked, gathered)
            })
            .await;
        let (packet, anchor, roof, baked, gathered) = built;
        let log_bake = baked.map(|(ms, count)| (ms, count, roof.as_ref().map(|r| r.origin)));
        self.state_quiet(|s| {
            s.anchor = anchor;
            if s.game.enabled {
                s.game.roof = roof;
                if let Some((ms, _)) = baked {
                    s.game.roof_bake_ms = ms;
                    s.game.rebake = false;
                }
                if let Some((drawn, in_view)) = gathered {
                    s.game.drawn = drawn;
                    s.game.in_view = in_view;
                }
            }
        });
        if let Some((ms, count, Some(o))) = log_bake {
            let half = lighting::ROOF_CELLS as f32 * lighting::ROOF_CELL * 0.5;
            editor.log.info(format!(
                "* game lighting: roof map {0}x{0} around ({1:.0}, {2:.0}) from {count} objects in {ms} ms",
                lighting::ROOF_CELLS,
                o[0] + half,
                o[1] + half
            ));
        }
        Some(packet)
    }

    /// A change the pass itself publishes: no new pass for it.
    fn state_quiet(&self, f: impl FnOnce(&mut ViewState)) {
        f(&mut self.state.lock());
    }

    /// The linked game's configs and light animations Game Lighting reads.
    pub(crate) async fn game_data_for(
        &self,
        editor: &Arc<Editor>,
    ) -> (Option<Arc<gz_ltx::Ltx>>, Option<Arc<gz_fmt_xr::LightAnims>>) {
        let d = self.game_data(editor).await;
        (d.configs, d.anims)
    }

    async fn game_data(&self, editor: &Arc<Editor>) -> GameData {
        let Ok(link) = gz_game::GameLink::of(editor) else { return GameData::default() };
        let rev = link.rev();
        let cached = self.game_data.lock().clone();
        if cached.rev == Some(rev) {
            return cached;
        }
        let configs = link.configs().await.ok();
        let anims = match link.files().await {
            Ok(files) => editor
                .exec
                .io(move || files.read("lanims.xr").ok().and_then(|b| gz_fmt_xr::LightAnims::read(&b).ok()))
                .await
                .map(Arc::new),
            Err(_) => None,
        };
        let data = GameData { rev: Some(rev), configs, anims };
        *self.game_data.lock() = data.clone();
        data
    }

    /// Draws the scene now and reads the picture back, cropped to the render size
    /// (`screenshot_viewport`): a pass of its own, also while Level is hidden.
    pub async fn picture(self: &Arc<Self>, editor: &Arc<Editor>) -> Result<Image, CmdError> {
        let renderer =
            self.renderer_any(editor).ok_or_else(|| CmdError::new("no_window", "viewport RT unavailable"))?;
        self.settle().await;
        let packet = self
            .make_packet(editor, &renderer, false)
            .await
            .ok_or_else(|| CmdError::internal("viewport RT unavailable"))?;
        let serial = renderer.submit(packet.clone());
        {
            let mut b = self.build.lock();
            b.serial = serial;
            b.packet = Some(packet);
        }
        let r = renderer.clone();
        let image = editor
            .exec
            .io(move || r.capture(serial, Duration::from_secs(10)))
            .await
            .map_err(|e| CmdError::internal(format!("viewport RT unavailable: {e}")))?;
        editor.exec.wake().ping();
        Ok(image)
    }

    /// Reads the linked game's weather for the hour, its render settings and its weathers, off
    /// the UI thread (`Reload`, `SetWeather`). `reload` reads `user.ltx` again.
    pub async fn load_weather(self: &Arc<Self>, editor: &Arc<Editor>, reload: bool) {
        let (weather, hour, have_settings) = self.with(|s| {
            s.game.loading = true;
            (s.game.weather.clone(), s.game.hour, !reload && !s.game.weathers.is_empty())
        });
        let result = read_weather(editor, &weather, hour, !have_settings).await;
        self.with(|s| {
            s.game.loading = false;
            match result {
                Ok(r) => {
                    if let Some(settings) = r.settings {
                        s.game.settings = settings;
                    }
                    s.game.weathers = r.names;
                    s.game.weather = r.weather;
                    match r.at {
                        Ok(at) => {
                            s.game.at = at;
                            s.game.weather_ok = true;
                            s.game.error.clear();
                        }
                        Err(e) => {
                            s.game.weather_ok = false;
                            s.game.error = e;
                        }
                    }
                }
                Err(e) => {
                    s.game.weather_ok = false;
                    s.game.error = e;
                }
            }
        });
        editor.exec.wake().ping();
    }

    /// `frame_pacing`'s measure: the frames presented between two asks.
    pub(crate) fn pacing(&self, frame: u64) -> (f64, f64) {
        let now = Instant::now();
        let mut s = self.state.lock();
        let out = match s.pacing {
            Some((f, t)) if frame > f => {
                let dt = now.duration_since(t).as_secs_f64();
                let fps = (frame - f) as f64 / dt.max(1e-6);
                (fps, if fps > 0.0 { 1000.0 / fps } else { 0.0 })
            }
            _ => (0.0, 0.0),
        };
        s.pacing = Some((frame, now));
        out
    }
}

trait SceneOpt {
    fn of_opt(editor: &Editor) -> Option<Arc<SceneService>>;
}

impl SceneOpt for SceneService {
    /// The open project's scene service when there is one; nothing is made for a request that
    /// only draws.
    fn of_opt(editor: &Editor) -> Option<Arc<SceneService>> {
        editor.project_opt().and_then(|s| s.services.get::<SceneService>())
    }
}

/// What a weather read found.
struct WeatherRead {
    weather: String,
    names: Vec<String>,
    settings: Option<GameSettings>,
    at: Result<Weather, String>,
}

/// `LoadWeather` over the linked game's files: the list, the default for the hour, the file.
async fn read_weather(
    editor: &Arc<Editor>,
    weather: &str,
    hour: f32,
    settings: bool,
) -> Result<WeatherRead, String> {
    let link = gz_game::GameLink::of(editor).map_err(|e| e.to_string())?;
    let files = link.files().await.map_err(|e| e.to_string())?;
    let start_date = match link.configs().await {
        Ok(cfg) => cfg.r_string("alife", "start_date").ok().map(str::to_owned),
        Err(_) => None,
    };
    let root = link.root();
    let wanted = weather.to_owned();
    editor
        .exec
        .io(move || {
            let dir = "configs/environment/weathers/";
            let mut names: Vec<String> = files
                .find("configs/environment/weathers", "ltx")
                .into_iter()
                .filter_map(|f| {
                    f.path
                        .strip_prefix(dir)
                        .filter(|rest| !rest.contains('/'))
                        .map(|rest| rest.trim_end_matches(".ltx").to_owned())
                })
                .collect();
            names.sort();
            names.dedup();
            let settings = settings.then(|| {
                let path = PathBuf::from(&root).join("appdata").join("user.ltx");
                GameSettings::read(
                    &std::fs::read(path)
                        .map(|b| String::from_utf8_lossy(&b).into_owned())
                        .unwrap_or_default(),
                )
            });
            let name = if wanted.is_empty() {
                lighting::default_weather(&names, start_date.as_deref(), hour)
            } else {
                wanted
            };
            let at = if name.is_empty() {
                Err("the game has no weathers (configs\\environment\\weathers)".to_owned())
            } else {
                match files.read(&format!("{dir}{}.ltx", name.to_lowercase())) {
                    Ok(bytes) => lighting::weather_at(
                        &gz_ltx::LtxDoc::parse(&gz_base::text::cp1251_to_utf8(&bytes)),
                        &name,
                        hour,
                    ),
                    Err(_) => Err(format!("no weather '{name}' in the game's configs")),
                }
            };
            Ok(WeatherRead { weather: name, names, settings, at })
        })
        .await
}

/// Takes the picture `preview.png` is made of: the open scene once more from the viewport's
/// camera, at its size, with no grid, selection, gizmo, helpers or captions (a pass of its own,
/// `render_offscreen`). For a close that clears the scene, before it does; never in a UI pass.
pub async fn capture_preview(editor: &Arc<Editor>) -> Result<Image, String> {
    let vp = Viewport::of(editor);
    let renderer = vp.renderer_any(editor).ok_or_else(|| "the editor runs without a window".to_owned())?;
    let packet = vp
        .make_packet(editor, &renderer, true)
        .await
        .ok_or_else(|| "the scene could not be drawn".to_owned())?;
    // the packet's own assets may still be loading: give them a moment, as a thumbnail does
    let r = renderer.clone();
    let size = vp.read(|s| s.size);
    editor.exec.io(move || r.render_offscreen(&packet, size).map_err(|e| e.to_string())).await
}

/// What Place adds for a class: the Tools panel's pick (a library object, a spawn section, a
/// particle system). `None` clears it.
pub fn set_place_item(editor: &Editor, class: ObjClass, name: Option<&str>) {
    Viewport::of(editor).with(|s| match name.filter(|n| !n.is_empty()) {
        Some(n) => {
            s.place.insert(class, SmolStr::new(n));
        }
        None => {
            s.place.remove(&class);
        }
    });
}
