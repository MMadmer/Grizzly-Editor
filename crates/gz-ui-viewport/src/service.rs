//! [`Viewport`]: the view state the pane, the MCP handlers and the scene's keys share, the
//! renderer, and the scene pass - a packet built on the CPU pool whenever something it draws
//! changed, and never otherwise.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use glam::Vec3;
use gz_app::settings::{PrefDef, PrefType};
use gz_base::Rev;
use gz_editor::{CmdError, Editor, SmolStr};
use gz_render::{Color, FramePacket, Image, Renderer, View, ViewFlags, ViewMode as Shading};
use gz_scene::{ObjClass, SceneService};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::camera::Nav;
use crate::draw::{self, DrawOptions};
use crate::gizmo::{Anchor, GizmoType, Space};
use crate::lighting::{self, GameLight, GameSettings, Weather};
use crate::loader::SceneLoader;
use crate::session::{self, Saved};

/// How the viewport shades the scene: the View Mode pill.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ViewMode {
    /// A light along the view, a key light from above and an ambient: every surface reads.
    #[default]
    Studio,
    /// The textures as they are.
    Unlit,
    /// The weather and the scene's lights at an hour, per pixel, the way the game lights the level.
    Game,
    /// The edges of every mesh.
    Wireframe,
}

impl ViewMode {
    pub const ALL: [ViewMode; 4] = [ViewMode::Studio, ViewMode::Unlit, ViewMode::Game, ViewMode::Wireframe];

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Studio => "studio",
            Self::Unlit => "unlit",
            Self::Game => "game",
            Self::Wireframe => "wireframe",
        }
    }

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Studio => "Studio",
            Self::Unlit => "Unlit",
            Self::Game => "Game Lit",
            Self::Wireframe => "Wireframe",
        }
    }

    #[must_use]
    pub fn icon(self) -> gz_ui::Icon {
        match self {
            Self::Studio | Self::Game => gz_ui::Icon::Lit,
            Self::Unlit => gz_ui::Icon::Unlit,
            Self::Wireframe => gz_ui::Icon::Wireframe,
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.key().eq_ignore_ascii_case(text.trim()))
    }

    fn index(self) -> usize {
        Self::ALL.iter().position(|m| *m == self).unwrap_or(0)
    }

    fn of_index(i: usize) -> Self {
        Self::ALL.get(i).copied().unwrap_or_default()
    }
}

/// What the viewport draws besides the scene: the Show pill. One place for each switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Show {
    pub grid: bool,
    /// The frame the game's 4:3 screen shows.
    pub safe_rect: bool,
    pub stats: bool,
    /// Spawn points' flags, their shapes and boxes.
    pub spawn_helpers: bool,
    pub spawn_names: bool,
    /// The light controls' names over the lights.
    pub light_names: bool,
    pub glow_crosses: bool,
    /// The AI map's nodes.
    pub ai_map: bool,
    pub fog: bool,
}

impl Default for Show {
    fn default() -> Self {
        Self {
            grid: true,
            safe_rect: false,
            stats: false,
            spawn_helpers: true,
            spawn_names: false,
            light_names: false,
            glow_crosses: false,
            ai_map: true,
            fog: false,
        }
    }
}

/// A switch of the Show pill: its key (the command's parameter and the setting's) and where it
/// lives. Its title is its action's (`viewport.show_<key>`).
pub struct ShowFlag {
    pub key: &'static str,
    pub get: fn(&Show) -> bool,
    pub set: fn(&mut Show, bool),
}

/// The Show pill's switches in its order.
pub const SHOW_FLAGS: [ShowFlag; 9] = [
    ShowFlag { key: "grid", get: |s| s.grid, set: |s, v| s.grid = v },
    ShowFlag { key: "safe_rect", get: |s| s.safe_rect, set: |s, v| s.safe_rect = v },
    ShowFlag { key: "stats", get: |s| s.stats, set: |s, v| s.stats = v },
    ShowFlag { key: "spawn_helpers", get: |s| s.spawn_helpers, set: |s, v| s.spawn_helpers = v },
    ShowFlag { key: "spawn_names", get: |s| s.spawn_names, set: |s, v| s.spawn_names = v },
    ShowFlag { key: "light_names", get: |s| s.light_names, set: |s, v| s.light_names = v },
    ShowFlag { key: "glow_crosses", get: |s| s.glow_crosses, set: |s, v| s.glow_crosses = v },
    ShowFlag { key: "ai_map", get: |s| s.ai_map, set: |s, v| s.ai_map = v },
    ShowFlag { key: "fog", get: |s| s.fog, set: |s, v| s.fog = v },
];

/// The snapping pills: the steps' switches and how a placed or moved object meets a surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snaps {
    pub grid: bool,
    pub angle: bool,
    pub scale: bool,
    /// A placed or dropped object lands on the surface under the pointer (else on the grid's
    /// plane).
    pub surface: bool,
    /// ... and stands on its slope.
    pub align: bool,
    /// ... on the nearest vertex of what it lands on.
    pub vertex: bool,
}

impl Default for Snaps {
    fn default() -> Self {
        Self { grid: true, angle: true, scale: false, surface: true, align: false, vertex: false }
    }
}

/// The steps the snapping pills offer: metres, degrees, factors.
pub const MOVE_STEPS: [f32; 9] = [0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0];
pub const ANGLE_STEPS: [f32; 7] = [5.0, 10.0, 15.0, 22.5, 30.0, 45.0, 90.0];
pub const SCALE_STEPS: [f32; 6] = [0.05, 0.1, 0.25, 0.5, 1.0, 2.0];

/// The render size over the pane's (Preferences > Viewport > Render Scale), by its choice.
pub const RENDER_SCALES: [(f32, &str); 4] = [(0.25, "25 %"), (0.5, "50 %"), (1.0, "100 %"), (2.0, "200 %")];

/// The settings the viewport keeps in the editor's settings file (`[preferences]`), read and set
/// by `app.prefs` like every other.
pub(crate) const SETTINGS: [PrefDef; 19] = [
    pdef("viewport_mode", PrefType::U32, 0.0, 0.0, 3.0),
    pdef("viewport_show_grid", PrefType::Bool, 1.0, 0.0, 1.0),
    pdef("viewport_show_safe_rect", PrefType::Bool, 0.0, 0.0, 1.0),
    pdef("viewport_show_stats", PrefType::Bool, 0.0, 0.0, 1.0),
    pdef("viewport_show_spawn_helpers", PrefType::Bool, 1.0, 0.0, 1.0),
    pdef("viewport_show_spawn_names", PrefType::Bool, 0.0, 0.0, 1.0),
    pdef("viewport_show_light_names", PrefType::Bool, 0.0, 0.0, 1.0),
    pdef("viewport_show_glow_crosses", PrefType::Bool, 0.0, 0.0, 1.0),
    pdef("viewport_show_ai_map", PrefType::Bool, 1.0, 0.0, 1.0),
    pdef("viewport_show_fog", PrefType::Bool, 0.0, 0.0, 1.0),
    pdef("viewport_snap_grid", PrefType::Bool, 1.0, 0.0, 1.0),
    pdef("viewport_snap_angle", PrefType::Bool, 1.0, 0.0, 1.0),
    pdef("viewport_snap_scale", PrefType::Bool, 0.0, 0.0, 1.0),
    pdef("viewport_snap_surface", PrefType::Bool, 1.0, 0.0, 1.0),
    pdef("viewport_snap_align", PrefType::Bool, 0.0, 0.0, 1.0),
    pdef("viewport_snap_vertex", PrefType::Bool, 0.0, 0.0, 1.0),
    pdef("viewport_local_space", PrefType::Bool, 0.0, 0.0, 1.0),
    pdef("viewport_render_scale", PrefType::U32, 2.0, 0.0, 3.0),
    pdef("viewport_gpu_preference", PrefType::U32, 0.0, 0.0, 2.0),
];

const fn pdef(key: &'static str, ty: PrefType, default: f64, lo: f64, hi: f64) -> PrefDef {
    PrefDef { key, ty, default, lo, hi }
}

/// The key of a Show switch's setting.
#[must_use]
pub(crate) fn show_key(flag: &str) -> String {
    format!("viewport_show_{flag}")
}

/// Where a drag over the viewport would place what it carries: the ghost the pass draws.
#[derive(Clone, Debug, PartialEq)]
pub struct Ghost {
    pub what: crate::place::Placeable,
    pub position: Vec3,
    /// The surface's up where it lands, when the object stands on its slope.
    pub up: Option<Vec3>,
}

/// The painting brush the Paint mode's panel sets and the viewport paints with.
#[derive(Clone, Debug, PartialEq)]
pub struct Brush {
    /// Foliage: the detail mesh it paints (a mesh of the scene's detail layer).
    pub mesh: Option<SmolStr>,
    /// Foliage: metres. The AI map's brush is the map's own, in patches.
    pub radius: f32,
    /// Foliage: how dense, 0..1.
    pub density: f32,
}

impl Default for Brush {
    fn default() -> Self {
        Self { mesh: None, radius: 4.0, density: 1.0 }
    }
}

/// Game Lit: the weather and the hour the scene is lit by.
#[derive(Clone, Debug, PartialEq)]
pub struct GameLighting {
    /// Game Lit is the view mode (the settings' `viewport_mode`).
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

    /// What the frame lights with: Game Lit, with a weather that read.
    #[must_use]
    pub fn active(&self) -> bool {
        self.enabled && self.weather_ok
    }
}

/// The preferences and the viewport's own settings the view follows, as last applied.
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
    /// Tools > Box Selection > Limited Depth and Depth Tolerance.
    pub bp_lim_depth: bool,
    pub bp_depth_tolerance: f32,
    pub mode: ViewMode,
    pub show: Show,
    pub snaps: Snaps,
    pub space: Space,
    /// The render size over the pane's.
    pub render_scale: f32,
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
            snap_angle: 15.0,
            snap_scale: 0.25,
            bp_lim_depth: true,
            bp_depth_tolerance: 0.1,
            mode: ViewMode::default(),
            show: Show::default(),
            snaps: Snaps::default(),
            space: Space::World,
            render_scale: 1.0,
        }
    }
}

impl Prefs {
    fn read(editor: &Editor) -> Self {
        let s = gz_app::EditorSettings::of(editor);
        let d = Self::default();
        let f = |k: &str, def: f32| s.pref(k).map_or(def, |v| v as f32);
        let on = |k: &str, def: bool| s.pref(k).map_or(def, |v| v != 0.0);
        let mut show = d.show;
        for flag in &SHOW_FLAGS {
            (flag.set)(&mut show, on(&show_key(flag.key), (flag.get)(&d.show)));
        }
        let snaps = Snaps {
            grid: on("viewport_snap_grid", d.snaps.grid),
            angle: on("viewport_snap_angle", d.snaps.angle),
            scale: on("viewport_snap_scale", d.snaps.scale),
            surface: on("viewport_snap_surface", d.snaps.surface),
            align: on("viewport_snap_align", d.snaps.align),
            vertex: on("viewport_snap_vertex", d.snaps.vertex),
        };
        let scale_i = s.pref("viewport_render_scale").map_or(2, |v| v as usize);
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
            bp_lim_depth: on("bp_lim_depth", d.bp_lim_depth),
            bp_depth_tolerance: f("bp_depth_tolerance", d.bp_depth_tolerance),
            mode: ViewMode::of_index(s.pref("viewport_mode").map_or(0, |v| v as usize)),
            show,
            snaps,
            space: if on("viewport_local_space", false) { Space::Local } else { Space::World },
            render_scale: RENDER_SCALES.get(scale_i).map_or(1.0, |(v, _)| *v),
        }
    }

    /// The tools' rotation per point of an edge-on ring's drag.
    #[must_use]
    pub fn tools_sr(&self) -> f32 {
        0.02 * self.tools_sens_rot * self.tools_sens_rot
    }
}

/// Everything the view is: one lock, never held across an await.
#[derive(Clone, Debug)]
pub struct ViewState {
    pub nav: Nav,
    /// The transform tool (Select, Move, Rotate, Scale): what the gizmo is.
    pub gizmo: GizmoType,
    pub game: GameLighting,
    pub prefs: Prefs,
    /// The render size in pixels the pane last laid out (1280 x 720 before it ever did).
    pub size: [u32; 2],
    /// The gizmo's anchor of the selection, as the last scene pass found it.
    pub anchor: Option<Anchor>,
    /// What Place mode places, and the ghost of a drag over the viewport.
    pub armed: Option<crate::place::Placeable>,
    pub ghost: Option<Ghost>,
    /// The Paint mode's brush, and the layer Paint goes back to.
    pub brush: Brush,
    pub paint_layer: gz_scene::SceneMode,
    /// A short line the pane shows for a moment (the camera's speed).
    pub status: Option<(String, Instant)>,
    /// What is behind the scene: the theme's canvas colour, as the pane last drew it.
    pub background: Color,
    /// The theme's lines over the scene: the grid, its every tenth line, the Wireframe edges.
    pub scene_lines: [Color; 3],
    /// The pane as last laid out: its size in points and the points' pixels (`viewport.gesture`).
    pub pane_pt: [f32; 2],
    pub ppp: f32,
    scene_key: Option<String>,
    cameras: FxHashMap<String, Saved>,
    cameras_for: Option<PathBuf>,
    cameras_read: bool,
    saved_pose: Option<Saved>,
    pose_changed: Option<Instant>,
    pacing: Option<(u64, Instant, PassCount, Causes)>,
}

/// The frame clock's frames by cause (`gz_editor::FrameStamp::cause`).
type Causes = std::collections::BTreeMap<SmolStr, u64>;

/// What [`Viewport::pacing`] measured since the last ask.
#[derive(Clone, Debug, Default)]
pub(crate) struct Pacing {
    pub fps: f64,
    pub frame_ms: f64,
    /// The frame the last ask saw; `None` at the first ask.
    pub since: Option<u64>,
    pub seconds: f64,
    /// Scene passes built in that time, and by reason (only the reasons that occurred).
    pub passes: u64,
    pub by_reason: Vec<(Reason, u64)>,
    /// The last pass ever built and when.
    pub last_pass: Option<(Reason, Instant)>,
    /// The UI frames in that time by what they were drawn for, the most first.
    pub causes: Vec<(SmolStr, u64)>,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            nav: Nav::default(),
            gizmo: GizmoType::None,
            game: GameLighting::default(),
            prefs: Prefs::default(),
            size: [1280, 720],
            anchor: None,
            armed: None,
            ghost: None,
            brush: Brush::default(),
            paint_layer: gz_scene::SceneMode::Details,
            status: None,
            // the kit's tokens `canvas` (151419) and `scene.*` until the pane draws with the theme's
            background: Color::rgb(0x15, 0x14, 0x19),
            scene_lines: [
                Color::rgb(0x90, 0x90, 0x90),
                Color::rgb(0xb4, 0xb4, 0xb4),
                Color::rgb(0xd2, 0xd2, 0xd2),
            ],
            pane_pt: [0.0, 0.0],
            ppp: 1.0,
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
    /// The step of the grid, scale or angle snap (the preferences).
    #[must_use]
    pub fn step(&self, kind: GizmoType) -> f32 {
        match kind {
            GizmoType::Move => self.prefs.snap_grid,
            GizmoType::Scale => self.prefs.snap_scale,
            GizmoType::Rotate => self.prefs.snap_angle,
            GizmoType::None => 0.0,
        }
    }

    /// Whether the grid, scale or angle snap is on.
    #[must_use]
    pub fn snap_on(&self, kind: GizmoType) -> bool {
        match kind {
            GizmoType::Move => self.prefs.snaps.grid,
            GizmoType::Scale => self.prefs.snaps.scale,
            GizmoType::Rotate => self.prefs.snaps.angle,
            GizmoType::None => false,
        }
    }

    /// The step a drag of `kind` snaps to, when its snap is on.
    #[must_use]
    pub fn snap(&self, kind: GizmoType) -> Option<f32> {
        self.snap_on(kind).then(|| self.step(kind))
    }

    /// `aspect` as the camera's maths use it: height over width.
    #[must_use]
    pub fn aspect(&self) -> f32 {
        if self.size[0] == 0 { 1.0 } else { self.size[1] as f32 / self.size[0] as f32 }
    }

    /// What a scene pass reads of the view state. Two states equal here give the same picture;
    /// the bookkeeping beside it (the cameras kept per scene and the pose's save timer, the
    /// status line, the armed thing, the pane's points) and what a pass publishes itself (the
    /// anchor, the game lights drawn, the roof map) are left out.
    fn drawn(&self) -> Drawn {
        let g = &self.game;
        Drawn {
            nav: self.nav.clone(),
            prefs: self.prefs,
            size: self.size,
            background: self.background,
            scene_lines: self.scene_lines,
            ghost: self.ghost.clone(),
            game: GameDrawn {
                enabled: g.enabled,
                weather: g.weather.clone(),
                hour: g.hour,
                exposure: [g.exposure, g.auto_exposure, g.scene_lum, g.indoor_hemi],
                at: g.at.clone(),
                weather_ok: g.weather_ok,
                settings: g.settings,
                rebake: g.rebake,
            },
        }
    }
}

/// [`ViewState::drawn`].
#[derive(PartialEq)]
struct Drawn {
    nav: Nav,
    prefs: Prefs,
    size: [u32; 2],
    background: Color,
    scene_lines: [Color; 3],
    ghost: Option<Ghost>,
    game: GameDrawn,
}

/// What a pass reads of Game Lighting.
#[derive(PartialEq)]
struct GameDrawn {
    enabled: bool,
    weather: String,
    hour: f32,
    exposure: [f32; 4],
    at: Weather,
    weather_ok: bool,
    settings: GameSettings,
    rebake: bool,
}

/// What a scene pass is built from: one revision per input. A pass is built when one of them
/// moved since the last pass ([`due`]) and never otherwise - the pane shows the picture it has.
#[derive(Clone, Debug, PartialEq)]
struct BuildKey {
    /// The scene's snapshot: objects, selection, the tools' data.
    scene: Rev,
    library: Rev,
    /// The view state as a pass reads it ([`ViewState::drawn`]): the camera, the view options,
    /// the pane's size and the theme's canvas, Game Lighting; [`Viewport::touch`] for an input
    /// outside it.
    view: u64,
    /// The meshes and textures loaded since a pass that drew placeholders.
    assets: u64,
    /// Moves on every ask while something in view animates (a particle system playing, a motion,
    /// View > Real Time): each frame a pass, only then.
    anim: u64,
    /// A spawn's rig arrived or its playback changed.
    spawn: u64,
}

/// Why a scene pass was built (`perf.frames` counts them by this).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reason {
    /// The first pass of the renderer.
    First,
    Scene,
    Library,
    View,
    /// Meshes or textures a pass waited for arrived.
    Assets,
    /// Something in view animates.
    Animation,
    Spawn,
    /// A texture the shown picture draws was loaded again.
    Stale,
    /// A picture asked over MCP (`shot.viewport`, Game Lit's lights in `viewport.view`).
    Picture,
    /// The project's `preview.png`.
    Preview,
}

impl Reason {
    pub const ALL: [Reason; 10] = [
        Reason::First,
        Reason::Scene,
        Reason::Library,
        Reason::View,
        Reason::Assets,
        Reason::Animation,
        Reason::Spawn,
        Reason::Stale,
        Reason::Picture,
        Reason::Preview,
    ];

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Reason::First => "first",
            Reason::Scene => "scene",
            Reason::Library => "library",
            Reason::View => "view",
            Reason::Assets => "assets",
            Reason::Animation => "animation",
            Reason::Spawn => "spawn",
            Reason::Stale => "stale",
            Reason::Picture => "picture",
            Reason::Preview => "preview",
        }
    }
}

/// Whether the pane needs a new pass, and why: the first input of `now` whose revision moved
/// since `last` was built, or the packet shown going stale. `None`: nothing it shows changed.
fn due(last: Option<&BuildKey>, now: &BuildKey, stale: bool) -> Option<Reason> {
    let Some(l) = last else { return Some(Reason::First) };
    let moved = [
        (l.scene != now.scene, Reason::Scene),
        (l.library != now.library, Reason::Library),
        (l.view != now.view, Reason::View),
        (l.assets != now.assets, Reason::Assets),
        (l.anim != now.anim, Reason::Animation),
        (l.spawn != now.spawn, Reason::Spawn),
        (stale, Reason::Stale),
    ];
    moved.into_iter().find(|(m, _)| *m).map(|(_, r)| r)
}

/// The passes built since the viewport was made, by [`Reason`], and the last one.
#[derive(Clone, Debug, Default)]
pub(crate) struct PassCount {
    pub total: u64,
    pub by_reason: [u64; Reason::ALL.len()],
    pub last: Option<(Reason, Instant)>,
}

impl PassCount {
    fn add(&mut self, why: Reason) {
        self.total += 1;
        if let Some(i) = Reason::ALL.iter().position(|r| *r == why) {
            self.by_reason[i] += 1;
        }
        self.last = Some((why, Instant::now()));
    }
}

#[derive(Default)]
struct BuildCtl {
    in_flight: bool,
    again: bool,
    last: Option<BuildKey>,
    packet: Option<Arc<FramePacket>>,
    serial: u64,
    /// Passes that ended: `settle` waits for the one under way, not for a moment without one.
    finished: u64,
    passes: PassCount,
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
    /// The thread that makes the renderer, while it runs.
    renderer_start: Mutex<Option<gz_task::Started>>,
    renderer_starting: AtomicBool,
    build: Mutex<BuildCtl>,
    loader_key: Mutex<Option<(Option<PathBuf>, Rev)>>,
    game_data: Mutex<GameData>,
    /// The pane's own frame rate and the renderer's, for the stats pill.
    pub(crate) rates: Mutex<Rates>,
    /// The last pass found a particle system in view playing.
    animating: AtomicBool,
    anim_tick: AtomicU64,
    /// A gesture fed over MCP (`viewport.gesture`): the pane takes one step of it a frame.
    pub(crate) gesture: Mutex<Option<crate::gesture::Pending>>,
    /// The tasks the pane's gestures started that have not ended.
    pub(crate) tracked: std::sync::atomic::AtomicUsize,
}

/// A picture of the window holds for the viewport's pass of the scene, when Level is what it
/// shows.
fn set_gate(editor: &Editor) {
    gz_app::PictureGate::of(editor).set(|ed| {
        Box::pin(async move {
            if ed.frames.current().active_tab != "level" {
                return Vec::new();
            }
            Viewport::of(&ed).current(&ed, PICTURE_LIMIT).await
        })
    });
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
        editor.services.get_or_init(|| {
            // the viewport's own settings, as the last session left them
            let settings = gz_app::EditorSettings::of(editor);
            settings.add_defs(&SETTINGS);
            let mut state = ViewState { prefs: Prefs::read(editor), ..ViewState::default() };
            state.game.enabled = state.prefs.mode == ViewMode::Game;
            Viewport {
                state: Mutex::new(state),
                rev: AtomicU64::new(1),
                renderer: Mutex::new(None),
                renderer_failed: Mutex::new(false),
                renderer_start: Mutex::new(None),
                renderer_starting: AtomicBool::new(false),
                build: Mutex::new(BuildCtl::default()),
                loader_key: Mutex::new(None),
                game_data: Mutex::new(GameData::default()),
                rates: Mutex::new(Rates::default()),
                animating: AtomicBool::new(false),
                anim_tick: AtomicU64::new(0),
                gesture: Mutex::new(None),
                tracked: std::sync::atomic::AtomicUsize::new(0),
            }
        })
    }

    /// The pane as it was laid out this frame; no pass is asked for it.
    pub(crate) fn set_pane(&self, size_pt: [f32; 2], ppp: f32) {
        self.state_quiet(|s| {
            s.pane_pt = size_pt;
            s.ppp = ppp;
        });
    }

    /// Whether the last pass found a particle system in view playing (`perf.frames`'
    /// `realtime_render`).
    #[must_use]
    pub fn animating(&self) -> bool {
        self.animating.load(Ordering::Acquire)
    }

    /// Changes the view state; when the change reaches what a pass draws
    /// ([`ViewState::drawn`]), the next scene pass draws it. A change of the bookkeeping alone
    /// asks for no pass.
    pub fn with<R>(&self, f: impl FnOnce(&mut ViewState) -> R) -> R {
        let mut s = self.state.lock();
        let before = s.drawn();
        let r = f(&mut s);
        if s.drawn() != before {
            self.rev.fetch_add(1, Ordering::AcqRel);
        }
        r
    }

    /// Asks for a scene pass for an input outside the view state (Real Time switched, the
    /// textures reloaded, a spawn's playback).
    pub fn touch(&self) {
        self.rev.fetch_add(1, Ordering::AcqRel);
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

    /// The renderer for the UI thread: the first call starts it on a thread of its own and
    /// answers `None` until it is up. Its pipelines take hundreds of milliseconds to compile
    /// (412 ms measured on the Level tab's first frame), which no frame waits for.
    pub fn start_renderer(
        &self,
        editor: &Arc<Editor>,
        gpu: &egui_wgpu::RenderState,
    ) -> Option<Arc<Renderer>> {
        if let Some(r) = self.renderer() {
            return Some(r);
        }
        if *self.renderer_failed.lock() || self.renderer_starting.swap(true, Ordering::AcqRel) {
            return None;
        }
        let (ed, device, queue) = (editor.clone(), gpu.device.clone(), gpu.queue.clone());
        // asked for in the Level tab's first frame: the pool makes the thread
        let started = editor.exec.thread("gz-render-start", move || {
            let me = Viewport::of(&ed);
            let made = Renderer::new(device, queue, gz_app::gpu_gate(&ed), ed.exec.clone());
            me.install(&ed, made);
            me.renderer_starting.store(false, Ordering::Release);
        });
        *self.renderer_start.lock() = Some(started);
        // a picture taken meanwhile waits for it
        set_gate(editor);
        // the window's device goes after the start has ended
        let me = Viewport::of(editor);
        gz_app::GpuOwners::of(editor).on_exit("the viewport's renderer start", move || me.join_start());
        None
    }

    /// The renderer is being made.
    pub(crate) fn renderer_starting(&self) -> bool {
        self.renderer_starting.load(Ordering::Acquire)
    }

    fn join_start(&self) {
        let started = self.renderer_start.lock().take();
        if let Some(started) = started {
            started.join();
        }
    }

    /// The renderer on the device the window draws with, made the first time, for a caller that
    /// may wait (a command): one the UI thread started is waited for.
    pub fn ensure_renderer(&self, editor: &Editor, gpu: &egui_wgpu::RenderState) -> Option<Arc<Renderer>> {
        while self.renderer_starting.load(Ordering::Acquire) {
            self.join_start();
            if self.renderer_starting.load(Ordering::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        if self.renderer().is_none() && !*self.renderer_failed.lock() {
            let made = Renderer::new(
                gpu.device.clone(),
                gpu.queue.clone(),
                gz_app::gpu_gate(editor),
                editor.exec.clone(),
            );
            self.install(editor, made);
        }
        self.renderer()
    }

    /// Keeps a renderer just made (the first one wins), or remembers that it failed.
    fn install(&self, editor: &Editor, made: gz_render::Result<Renderer>) {
        let made = match made {
            Ok(made) => made,
            Err(e) => {
                *self.renderer_failed.lock() = true;
                editor.log.error(format!("the viewport's renderer did not start: {e}"));
                return;
            }
        };
        {
            let mut r = self.renderer.lock();
            if r.is_some() {
                drop(r);
                made.shutdown();
                return;
            }
            *r = Some(Arc::new(made));
        }
        set_gate(editor);
        // the window stops the render thread while its loop still runs
        let me = Viewport::of(editor);
        gz_app::GpuOwners::of(editor).on_exit("the viewport's renderer", move || {
            let taken = me.renderer.lock().take();
            if let Some(r) = taken {
                r.shutdown();
            }
        });
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
            let read_weather = self.with(|s| {
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
                s.game.enabled = prefs.mode == ViewMode::Game;
                s.game.enabled && !s.game.weather_ok && !s.game.loading
            });
            // Game Lit reads the linked game's weather the first time it comes up
            if read_weather {
                let (vp, ed) = (Viewport::of(editor), editor.clone());
                self.with(|s| s.game.loading = true);
                editor.exec.spawn(async move { vp.load_weather(&ed, true).await });
            }
        }
        self.sync_loader(editor);
        self.sync_scene_camera(editor);
        self.sync_render(editor);
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
        let editor_root = gz_project::library_root(editor);
        editor.exec.spawn(async move {
            let game = match gz_game::GameLink::of(&ed) {
                Ok(g) => g.files().await.ok(),
                Err(_) => None,
            };
            let scene = ed.clone();
            let library: crate::loader::LibrarySource =
                Arc::new(move || SceneService::of_opt(&scene).map(|svc| svc.library.clone()));
            let media = Arc::downgrade(&ed);
            let textures: crate::loader::TextureAssets =
                Arc::new(move |name: &str| gz_media::Media::get(media.upgrade()?.as_ref())?.texture_file(name));
            let loader =
                Arc::new(SceneLoader::new(project, editor_root, game).with_library(library).with_textures(textures));
            crate::spawn_anim::SpawnAnims::of(&ed).set_loader(loader.clone());
            renderer.assets().set_loader(Some(loader));
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

    /// Keeps the cameras of every scene (and their bookmarks) in the project's session, off the
    /// UI thread.
    fn write_cameras(&self, editor: &Arc<Editor>) {
        let Some(root) = editor.project_root() else { return };
        let text = self.read(|s| session::write(&s.cameras));
        let file = root.join("temp").join(session::FILE);
        let log = editor.log.clone();
        editor.exec.spawn_io(move || {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Err(e) = gz_editor::fsx::write_atomic(&file, text.as_bytes()) {
                log.warn(format!("the viewport's cameras could not be kept in {}: {e}", file.display()));
            }
        });
    }

    /// Keeps the camera in bookmark `n` of the open scene; `false` without one.
    pub(crate) fn store_bookmark(&self, editor: &Arc<Editor>, n: u8) -> bool {
        let stored = self.with(|s| {
            let key = s.scene_key.clone()?;
            let pose = Saved { hpb: s.nav.hpb, position: s.nav.position };
            s.cameras.insert(session::bookmark_key(&key, n), pose);
            Some(())
        });
        if stored.is_some() {
            self.write_cameras(editor);
        }
        stored.is_some()
    }

    /// Puts the camera where bookmark `n` of the open scene keeps it; `false` when it keeps none.
    pub(crate) fn go_to_bookmark(&self, n: u8) -> bool {
        self.with(|s| {
            let key = s.scene_key.clone()?;
            let c = s.cameras.get(&session::bookmark_key(&key, n)).copied()?;
            s.nav.set(c.hpb, c.position);
            s.nav.target = c.position + s.nav.direction() * 10.0;
            s.nav.reset_state();
            Some(())
        })
        .is_some()
    }

    /// Asks for a scene pass when anything it draws changed ([`due`]); otherwise the pane keeps
    /// the picture it has. Never blocks: the pass is built on the CPU pool and handed to the
    /// render thread.
    pub fn request_build(self: &Arc<Self>, editor: &Arc<Editor>) {
        let Some(renderer) = self.renderer() else { return };
        let key = self.key(editor, &renderer);
        let why = {
            let mut b = self.build.lock();
            let stale = b.packet.as_ref().is_some_and(|p| p.is_stale(renderer.assets()));
            let Some(why) = due(b.last.as_ref(), &key, stale) else { return };
            if b.in_flight {
                b.again = true;
                return;
            }
            b.in_flight = true;
            b.again = false;
            why
        };
        let vp = self.clone();
        let ed = editor.clone();
        editor.exec.spawn(async move {
            let made = vp.make_packet(&ed, &renderer, false, why).await;
            let again = {
                let mut b = vp.build.lock();
                b.in_flight = false;
                b.finished += 1;
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
        // View > Real Time draws every frame too
        let anim = if self.animating.load(Ordering::Acquire) || real_time(editor) {
            self.anim_tick.fetch_add(1, Ordering::AcqRel) + 1
        } else {
            0
        };
        BuildKey {
            scene,
            library,
            view: self.rev(),
            assets: if stale { renderer.assets().generation() } else { 0 },
            anim,
            spawn: crate::spawn_anim::SpawnAnims::of(editor).rev(),
        }
    }

    /// Whether the last pass that reached the renderer drew the scene at `rev` or a newer one: the
    /// gizmo's anchor and what the picture shows are of that scene.
    pub(crate) fn drawn_scene(&self, rev: Rev) -> bool {
        self.build.lock().last.as_ref().is_some_and(|k| k.scene >= rev)
    }

    /// Waits for the pass being built, if one is: a picture asked over MCP draws the newest
    /// state. Only that pass - while something plays a new one follows at once, and a wait for a
    /// moment without any would last its whole limit.
    pub async fn settle(&self) {
        let started = {
            let b = self.build.lock();
            if !b.in_flight {
                return;
            }
            b.finished
        };
        for _ in 0..400 {
            tokio::time::sleep(Duration::from_millis(5)).await;
            if self.build.lock().finished > started {
                return;
            }
        }
    }

    /// Builds a packet of the scene as the view stands now (`preview`: the picture of the scene
    /// alone), counted under `why`. Publishes what the pass found: the gizmo's anchor, the lights
    /// drawn, the roof map.
    pub(crate) async fn make_packet(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        renderer: &Arc<Renderer>,
        preview: bool,
        why: Reason,
    ) -> Option<Arc<FramePacket>> {
        self.make_packet_from(editor, renderer, preview, why, None).await
    }

    /// [`Self::make_packet`], or with `shot` a packet of the scene from a camera of the
    /// picture's own: the view's state is read and nothing of the pass is published into it.
    pub(crate) async fn make_packet_from(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        renderer: &Arc<Renderer>,
        preview: bool,
        why: Reason,
        shot: Option<crate::shots::ShotView>,
    ) -> Option<Arc<FramePacket>> {
        self.build.lock().passes.add(why);
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
        let parts = crate::draw_particles::Particles::of(editor);
        let extras = crate::draw_extra::Extras::of(editor);
        // the particle library is read only for a scene that has particle systems
        let has_particles = snap.as_ref().is_some_and(|s| s.state.count(ObjClass::Particles) > 0);
        let particles = if has_particles { parts.library(editor).await } else { None };
        // a motion playing: its object where the motion has it now
        let posed = snap.as_ref().and_then(|s| motion_pose(editor, &s.state));
        let playing = posed.is_some();
        if let Some((_, Some((hpb, at)))) = &posed {
            let (hpb, at) = (*hpb, *at);
            self.state_quiet(|s| s.nav.set(hpb, at));
        }
        let posed = posed.map(|(st, _)| st);
        let anims = crate::spawn_anim::SpawnAnims::of(editor);
        let ed = editor.clone();
        let built = editor
            .exec
            .cpu(move || {
                let cam = shot.map_or_else(|| st.nav.camera(), |s| s.camera);
                // the render scale sizes the picture; the picture of the project keeps the pane's
                let size = match shot {
                    Some(s) => s.size,
                    None if preview => st.size,
                    None => scaled(st.size, st.prefs.render_scale),
                };
                // a picture of the scene alone: the project's, or a shot that asked for it
                let preview = preview || shot.is_some_and(|s| s.clean);
                let show = st.prefs.show;
                let mut view = View::new(cam, size);
                view.background = st.background;
                view.mode = match st.prefs.mode {
                    ViewMode::Wireframe => Shading::Wireframe,
                    ViewMode::Unlit => Shading::Unlit,
                    ViewMode::Game if st.game.active() => Shading::GameLighting,
                    // Game Lit lights as the studio does until its weather has been read
                    ViewMode::Game | ViewMode::Studio => Shading::Lit,
                };
                view.flags = ViewFlags::HELPERS | ViewFlags::SELECTION;
                view.style = gz_render::ViewStyle { fog: show.fog };
                if show.grid {
                    view.flags |= ViewFlags::GRID;
                }
                view.grid.cells = st.prefs.grid_count.max(1);
                view.grid.cell_size = st.prefs.grid_size.max(0.01);
                [view.grid.color, view.grid.thick_color, view.wire_color] = st.scene_lines;
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
                let state = posed.as_ref().or(snap.as_ref().map(|s| &s.state)).unwrap_or(&empty_state);
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
                let opts = DrawOptions {
                    helpers: show.spawn_helpers && !preview,
                    spawn_names: show.spawn_names,
                    light_names: show.light_names,
                    glow_crosses: show.glow_crosses,
                    ghost: if preview || shot.is_some() { None } else { st.ghost.clone() },
                    game,
                };
                let mut built = draw::build(state, library, &assets, &opts);
                // the spawns' models in their startup animation's pose
                let poses = anims.poses(&ed, state);
                if !poses.by_object.is_empty() {
                    for (inst, owner) in built.list.instances.iter_mut().zip(&built.owners) {
                        if let Some(p) = poses.by_object.get(owner) {
                            inst.pose = Some(p.clone());
                        }
                    }
                }
                let animating = crate::draw_extra::extend(
                    &extras,
                    &parts,
                    &crate::draw_extra::Input {
                        state,
                        lib: library,
                        pick: pick.as_deref(),
                        assets: &assets,
                        camera: &cam,
                        size,
                        particles,
                        ai_map_shown: show.ai_map,
                    },
                    &mut built,
                );
                let packet = match &pick {
                    Some(pick) => {
                        let reach = draw::frustum_reach(&vp_m);
                        let vis = draw::visibility(&built, state, pick, &assets, &frustum, &reach);
                        gz_render::build_packet_with(&built.list, &view, &assets, Some(&vis))
                    }
                    None => gz_render::build_packet(&built.list, &view, &assets),
                };
                let anchor = Anchor::of(&draw::selection_gizmo_points(state, library));
                let animating = animating || poses.playing;
                (Arc::new(packet), anchor, roof, baked, gathered, animating)
            })
            .await;
        let (packet, anchor, roof, baked, gathered, animating) = built;
        if shot.is_some() {
            // the author's view keeps its anchor, its lights and its roof map
            return Some(packet);
        }
        let animating = animating || playing;
        self.animating.store(animating, Ordering::Release);
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
            editor.log.detail(format!(
                "game lighting: roof map {0}x{0} around ({1:.0}, {2:.0}) from {count} objects in {ms} ms",
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

    /// What a pass of the scene still waits for: placeholders drawn for a mesh still to come and
    /// textures of what it draws still loading. Assets of what lies outside the view load on
    /// meanwhile and are not waited for: the picture does not show them.
    pub(crate) fn pending_of(renderer: &Renderer, packet: &FramePacket) -> Vec<String> {
        let mut out = Vec::new();
        if packet.waiting_on_assets() {
            out.push("meshes still loading (placeholders drawn)".to_owned());
        }
        if packet.textures_loading(renderer.assets()) {
            out.push("textures of the view still loading".to_owned());
        }
        out
    }

    /// Holds until the window's viewport has shown the scene as the caller's commands left it: the
    /// pass for the scene's revision built and a frame of the render thread that drew it (or a
    /// newer pass) with nothing in view still loading - or `limit` ran out. Answers what was still
    /// pending then (empty: the picture is current). Without a renderer there is nothing to wait for.
    pub async fn current(self: &Arc<Self>, editor: &Arc<Editor>, limit: Duration) -> Vec<String> {
        let deadline = Instant::now() + limit;
        // a renderer the pane started is waited for
        while self.renderer().is_none() && self.renderer_starting() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
        let Some(renderer) = self.renderer() else {
            return if self.renderer_starting() {
                vec!["the viewport's renderer".to_owned()]
            } else {
                Vec::new()
            };
        };
        let rev = SceneService::of_opt(editor).map_or(Rev::ZERO, |svc| svc.store.rev());
        let view = self.rev();
        // the pass that drew the caller's scene and view: a finished pass of their revisions or
        // newer ones, whatever runs meanwhile - while a particle system plays a new pass follows
        // each one
        let mut drawn: Option<u64> = None;
        loop {
            self.request_build(editor);
            {
                let b = self.build.lock();
                if drawn.is_none() && b.last.as_ref().is_some_and(|k| k.scene >= rev && k.view >= view) {
                    drawn = Some(b.serial);
                }
            }
            // the render thread draws a packet again when what it waited for arrives, and a pass
            // whose placeholders became meshes follows: the newest frame says what is still missing
            let frame = renderer.finished();
            let pending = match drawn {
                None => vec!["the viewport's pass of the scene".to_owned()],
                Some(serial) if frame.packet < serial => vec!["the render thread's frame".to_owned()],
                Some(_) if frame.pending => vec!["meshes or textures of the view still loading".to_owned()],
                Some(_) => Vec::new(),
            };
            if pending.is_empty() || Instant::now() >= deadline {
                return pending;
            }
            // the pass after an asset arrived is built from a frame of the pane
            editor.exec.wake().ping();
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    }

    /// Draws the scene now and reads the picture back, cropped to the render size
    /// (`shot.viewport`): a pass of its own, also while Level is hidden. The pass is built
    /// again until nothing it draws is still loading (up to `limit`); `pending` says what still
    /// was when the limit ran out.
    pub async fn picture(self: &Arc<Self>, editor: &Arc<Editor>) -> Result<(Image, Vec<String>), CmdError> {
        let renderer = self.renderer_any(editor).ok_or_else(|| {
            CmdError::new("no_window", "the viewport draws nothing: the editor runs without a window")
        })?;
        self.settle().await;
        let deadline = Instant::now() + PICTURE_LIMIT;
        let (packet, pending) = loop {
            let packet = self
                .make_packet(editor, &renderer, false, Reason::Picture)
                .await
                .ok_or_else(|| CmdError::internal("the viewport could not draw the scene"))?;
            let pending = Self::pending_of(&renderer, &packet);
            if pending.is_empty() || Instant::now() >= deadline {
                break (packet, pending);
            }
            // what the pass asked for loads on the pools: build again once something arrived
            let generation = renderer.assets().generation();
            while renderer.assets().generation() == generation && Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        };
        let serial = renderer.submit(packet.clone());
        {
            let mut b = self.build.lock();
            b.serial = serial;
            b.packet = Some(packet);
        }
        let r = renderer.clone();
        let image =
            editor.exec.io(move || r.capture(serial, Duration::from_secs(10))).await.map_err(|e| {
                CmdError::internal(format!("the viewport's picture could not be read back: {e}"))
            })?;
        editor.exec.wake().ping();
        Ok((image, pending))
    }

    /// Reads the linked game's weather for the hour, its render settings and its weathers, off
    /// the UI thread. `reload` reads `user.ltx` again.
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

    /// `perf.frames`' measure: the frames presented between two asks, the frame the last ask
    /// saw (`None` at the first), and the scene passes built since then.
    pub(crate) fn pacing(&self, frame: u64, causes: Causes) -> Pacing {
        let now = Instant::now();
        let passes = self.build.lock().passes.clone();
        let mut s = self.state.lock();
        let mut out = Pacing { last_pass: passes.last, ..Pacing::default() };
        if let Some((f, t, before, causes_before)) = &s.pacing {
            let dt = now.duration_since(*t).as_secs_f64();
            out.since = Some(*f);
            out.seconds = dt;
            if frame > *f {
                out.fps = (frame - f) as f64 / dt.max(1e-6);
                out.frame_ms = if out.fps > 0.0 { 1000.0 / out.fps } else { 0.0 };
            }
            out.passes = passes.total - before.total;
            for (i, r) in Reason::ALL.iter().enumerate() {
                let n = passes.by_reason[i] - before.by_reason[i];
                if n > 0 {
                    out.by_reason.push((*r, n));
                }
            }
            out.causes = causes
                .iter()
                .map(|(k, n)| (k.clone(), n - causes_before.get(k).copied().unwrap_or(0)))
                .filter(|(_, n)| *n > 0)
                .collect();
            out.causes.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        }
        s.pacing = Some((frame, now, passes, causes));
        out
    }
}

pub(crate) trait SceneOpt {
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

/// The weather read from the linked game's files: the list, the default for the hour, the file.
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
        .make_packet(editor, &renderer, true, Reason::Preview)
        .await
        .ok_or_else(|| "the scene could not be drawn".to_owned())?;
    // the packet's own assets may still be loading: give them a moment, as a thumbnail does
    let r = renderer.clone();
    let size = vp.read(|s| s.size);
    editor.exec.io(move || r.render_offscreen(&packet, size).map_err(|e| e.to_string())).await
}

// ---- the viewport's settings; the camera other surfaces read ------------------------------------

/// The render size at a scale: at least a pixel a side.
fn scaled(size: [u32; 2], scale: f32) -> [u32; 2] {
    let q = if scale.is_finite() { scale.clamp(0.25, 2.0) } else { 1.0 };
    [((size[0] as f32 * q).round() as u32).max(1), ((size[1] as f32 * q).round() as u32).max(1)]
}

/// Real Time (Preferences > Viewport): the viewport draws every frame instead of when something
/// it shows changed.
pub(crate) fn real_time(editor: &Editor) -> bool {
    gz_app::EditorSettings::of(editor).render_flag("real_time")
}

/// The Reload Textures generation the viewport last loaded at.
#[derive(Default)]
struct ReloadSeen(AtomicU64);

/// The GPU preference the settings held when it was last applied; `u64::MAX` before the first
/// look, which only takes note of it.
struct GpuPreferenceSeen(AtomicU64);

impl Default for GpuPreferenceSeen {
    fn default() -> Self {
        Self(AtomicU64::new(u64::MAX))
    }
}

/// How long a picture waits for what its pass draws to load before it answers with `pending`.
pub(crate) const PICTURE_LIMIT: Duration = Duration::from_secs(15);

impl Viewport {
    /// Fills the camera slot quests read, once; applies a GPU preference changed in the settings;
    /// asks for the next frame while Real Time is on.
    fn sync_render(&self, editor: &Arc<Editor>) {
        let cam = gz_editor::ViewCamera::of(editor);
        if !cam.filled() {
            let weak = Arc::downgrade(editor);
            cam.provide(move || {
                let ed = weak.upgrade()?;
                let vp = ed.services.get::<Viewport>()?;
                let (position, direction) =
                    vp.read(|s| (s.nav.position.to_array(), s.nav.direction().to_array()));
                let level = SceneService::of_opt(&ed)
                    .map(|s| s.store.snapshot())
                    .filter(|s| s.valid)
                    .map(|s| s.meta.name())
                    .filter(|n| !n.is_empty());
                Some(gz_editor::CameraPose { position, direction: Some(direction), level })
            });
        }
        let settings = gz_app::EditorSettings::of(editor);
        let wanted = settings.pref("viewport_gpu_preference").map_or(0, |v| v as u64);
        let seen = editor.services.get_or_init(GpuPreferenceSeen::default);
        let before = seen.0.swap(wanted, Ordering::AcqRel);
        if before != u64::MAX && before != wanted {
            let hint = gz_app::gpu::DriverHint::ALL
                .get(wanted as usize)
                .copied()
                .unwrap_or(gz_app::gpu::DriverHint::Auto);
            gz_app::gpu::Gpu::of(editor).set_hint(editor, hint);
        }
        // Libraries > Reload Textures: a new loader of the same sources drops what was loaded
        let reload = gz_app::gpu::TextureReload::generation(editor);
        let seen = editor.services.get_or_init(ReloadSeen::default);
        if seen.0.swap(reload, Ordering::AcqRel) != reload {
            self.reload_assets(editor);
        }
        if real_time(editor)
            && let Ok(q) = gz_app::UiQueue::of(editor)
        {
            let fps = settings.pref("active_idle_fps").unwrap_or(120.0).clamp(30.0, 240.0);
            q.post(move |_, ctx| ctx.request_repaint_after(Duration::from_secs_f64(1.0 / fps)));
        }
    }

    /// Libraries > Reload Textures: the renderer drops what it loaded and reads it again when
    /// drawn (a new loader of the same sources).
    pub fn reload_assets(&self, editor: &Editor) {
        *self.loader_key.lock() = None;
        self.touch();
        editor.exec.wake().ping();
    }
}

/// Changes one of the viewport's settings and applies it at once; [`save`] writes the file.
pub(crate) fn set_setting(editor: &Arc<Editor>, key: &str, value: f64) {
    gz_app::EditorSettings::of(editor).set_pref(key, value);
    Viewport::of(editor).sync(editor);
    editor.exec.wake().ping();
}

/// Writes the editor's settings after a change: once per command, never once per value.
pub fn save(editor: &Editor) {
    gz_app::EditorSettings::of(editor).save_soon(editor);
}

/// The view mode.
pub fn set_mode(editor: &Arc<Editor>, mode: ViewMode) {
    set_setting(editor, "viewport_mode", mode.index() as f64);
}

/// A switch of the Show pill by its key; `false` for a key that names none.
pub fn set_show(editor: &Arc<Editor>, key: &str, on: bool) -> bool {
    if !SHOW_FLAGS.iter().any(|f| f.key == key) {
        return false;
    }
    set_setting(editor, &show_key(key), f64::from(u8::from(on)));
    true
}

/// A snapping switch: `grid`, `angle`, `scale`, `surface`, `align`, `vertex`.
pub fn set_snap(editor: &Arc<Editor>, key: &str, on: bool) {
    set_setting(editor, &format!("viewport_snap_{key}"), f64::from(u8::from(on)));
}

/// A snap's step: the preference the pill and Preferences > Tools > Snap share.
pub fn set_step(editor: &Arc<Editor>, kind: GizmoType, v: f32) {
    let key = match kind {
        GizmoType::Move => "snap_grid_step",
        GizmoType::Scale => "snap_scale_step",
        GizmoType::Rotate => "snap_angle_step",
        GizmoType::None => return,
    };
    set_setting(editor, key, f64::from(v));
}

/// The gizmo's space.
pub fn set_space(editor: &Arc<Editor>, space: Space) {
    set_setting(editor, "viewport_local_space", f64::from(u8::from(space == Space::Local)));
}

/// Mute Sounds (Preferences > Viewport).
pub fn set_mute_sounds(editor: &Editor, on: bool) {
    gz_app::gpu::Gpu::of(editor).set_mute_sounds(editor, on);
}

/// Real Time (Preferences > Viewport).
pub fn set_real_time(editor: &Editor, on: bool) {
    gz_app::EditorSettings::of(editor).set_render_flag("real_time", on);
    Viewport::of(editor).touch();
    editor.exec.wake().ping();
}

/// The Paint mode's brush.
pub fn set_brush(editor: &Editor, brush: Brush) {
    Viewport::of(editor).with(|s| s.brush = brush);
    editor.exec.wake().ping();
}

/// The camera bookmarks the open scene keeps.
#[must_use]
pub fn bookmarks(editor: &Editor) -> Vec<u8> {
    Viewport::of(editor).read(|s| {
        let Some(key) = &s.scene_key else { return Vec::new() };
        (1..=9u8).filter(|n| s.cameras.contains_key(&session::bookmark_key(key, *n))).collect()
    })
}

/// The Paint mode's brush as it is.
#[must_use]
pub fn brush(editor: &Editor) -> Brush {
    Viewport::of(editor).read(|s| s.brush.clone())
}

/// The scene with a playing motion's object posed where its motion has it now, and the camera
/// it gives when Camera View is on: `(-r.y, -r.x, -r.z)` from its position.
fn motion_pose(
    editor: &Editor,
    state: &gz_scene::SceneState,
) -> Option<(gz_scene::SceneState, Option<(Vec3, Vec3)>)> {
    let session = editor.project_opt()?;
    let play = (*gz_scene::cmd_edit::MotionPlayback::of(&session).playing.lock())?;
    let o = state.get(play.id)?;
    let raw = o.motion.clone()?;
    let m = raw.model()?;
    let p = gz_scene::objmotion::params(&m, &raw);
    let t = gz_scene::cmd_edit::MotionPlayback::time_of(&play, p.min_t, p.max_t);
    let (pos, rot) = gz_scene::objmotion::pose_at(&m, t);
    let mut moved = (**o).clone();
    moved.xform.position = pos;
    moved.xform.rotation = rot;
    let cam =
        (o.co_flags() & gz_scene::objmotion::CAMERA_VIEW != 0).then(|| (Vec3::new(rot.y, rot.x, rot.z), pos));
    let mut st = state.clone();
    st.replace(moved);
    Some((st, cam))
}

#[cfg(test)]
mod pass_tests {
    use super::{BuildKey, Reason, ViewState, due};
    use gz_base::Rev;

    fn key() -> BuildKey {
        BuildKey { scene: Rev::ZERO, library: Rev::ZERO, view: 1, assets: 0, anim: 0, spawn: 0 }
    }

    #[test]
    fn a_pass_is_due_only_when_an_input_moved() {
        let last = key();
        assert_eq!(due(None, &last, false), Some(Reason::First));
        assert_eq!(due(Some(&last), &key(), false), None, "nothing moved: the pane keeps its picture");
        assert_eq!(due(Some(&last), &BuildKey { view: 2, ..key() }, false), Some(Reason::View));
        assert_eq!(due(Some(&last), &BuildKey { anim: 5, ..key() }, false), Some(Reason::Animation));
        assert_eq!(due(Some(&last), &BuildKey { spawn: 1, ..key() }, false), Some(Reason::Spawn));
        assert_eq!(due(Some(&last), &key(), true), Some(Reason::Stale));
    }

    #[test]
    fn only_what_a_pass_reads_counts_as_a_change_of_the_view() {
        let a = ViewState::default();
        let mut b = a.clone();
        // the pose's save timer and the status line: bookkeeping, no new picture
        b.pose_changed = Some(std::time::Instant::now());
        b.status = Some(("speed".into(), std::time::Instant::now()));
        b.game.in_view = 7;
        assert!(a.drawn() == b.drawn());
        b.nav.position.x += 1.0;
        assert!(a.drawn() != b.drawn(), "the camera moved");
        let mut c = a.clone();
        c.size = [640, 480];
        assert!(a.drawn() != c.drawn(), "the pane's size");
    }
}

#[cfg(test)]
mod render_tests {
    use super::{Prefs, SHOW_FLAGS, Show, ViewMode, scaled};

    #[test]
    fn every_show_switch_has_a_key_a_title_and_its_own_field() {
        let mut keys: Vec<&str> = SHOW_FLAGS.iter().map(|f| f.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), SHOW_FLAGS.len());
        for f in &SHOW_FLAGS {
            let mut s = Show::default();
            let was = (f.get)(&s);
            (f.set)(&mut s, !was);
            assert_ne!(s, Show::default(), "{} sets its own field", f.key);
        }
        assert_eq!(ViewMode::parse("Game"), Some(ViewMode::Game));
        assert_eq!(Prefs::default().snap_angle, 15.0);
    }

    #[test]
    fn quality_scales_the_render_size() {
        assert_eq!(scaled([1280, 720], 0.5), [640, 360]);
        assert_eq!(scaled([1280, 720], 2.0), [2560, 1440]);
        assert_eq!(scaled([3, 3], 0.25), [1, 1]);
        assert_eq!(scaled([100, 100], f32::NAN), [100, 100]);
    }
}
