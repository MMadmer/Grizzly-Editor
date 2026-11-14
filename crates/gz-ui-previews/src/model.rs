//! The model preview: the `model` tab (docs/UI_DESIGN.md, section 6, "Model Preview").
//!
//! An orbit viewport of its own - a `gz_render::Renderer` whose target is sized to the canvas,
//! fed a draw list built here (the model, a floor grid under it, the bones when asked) - beside a
//! Details pane of what the model is made of.

use std::sync::Arc;

use egui::{
    Align, Color32, CornerRadius, Layout, PointerButton, Rect, ScrollArea, Sense, Ui, UiBuilder, Vec2, pos2,
};
use glam::{Mat4, Vec2 as GVec2, Vec3};
use gz_app::{Chord, DocCx, Document, OpenRequest, WindowLink};
use gz_editor::Editor;
use gz_render::{
    Camera, Color, DrawList, FramePacket, Instance, InstanceFlags, MeshHandle, Projection, Renderer, View,
    ViewFlags, ViewMode, build_packet, pick_ray,
};
use gz_ui::grid::read_only;
use gz_ui::splitter::splitter_tail;
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Kind, PropFlags, PropertyGrid, Toolbar, palette, state};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::image_pass::{ImageDraw, ImageMode, paint_image};
use crate::model_load::{ModelData, ModelSource, load};
use crate::pending::Pending;

/// pt (docs/UI_DESIGN.md, sections 2 and 6): the Details pane and what the viewport keeps.
pub const DETAILS_W: f32 = 320.0;
pub const SIDE_MIN: f32 = 220.0;
pub const CANVAS_MIN: f32 = 400.0;

/// Three-quarter shot from above, the framing the asset thumbnails use.
const START_OFFSET: Vec3 = Vec3::new(1.0, 0.8, -1.0);
const FOV_DEGREES: f32 = 45.0;
/// rad per point
const ORBIT_SPEED: f32 = 0.008;
/// ~89 degrees
const PITCH_LIMIT: f32 = 1.5533;
const JOINT_SIZE: f32 = 0.02;
const JOINT_COLOR: u32 = 0xffff_ff00;
const LINK_COLOR: u32 = 0xffa0_a000;
/// The floor: at most this many lines each way, and the margin round the model's box in radii.
const FLOOR_LINES_MAX: i64 = 48;
const FLOOR_MARGIN: f32 = 0.5;
/// The renderer's name for the model on show: a new one replaces it.
const MESH_NAME: &str = "gz-ui-previews\\model";

/// The orbit camera, all relative to the model's box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Orbit {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub pan: Vec3,
    pub center: Vec3,
    pub radius: f32,
}

impl Orbit {
    #[must_use]
    pub fn new(bounds: &gz_render::Aabb) -> Self {
        let mut o =
            Self { yaw: 0.0, pitch: 0.0, distance: 1.0, pan: Vec3::ZERO, center: Vec3::ZERO, radius: 1.0 };
        o.reset(bounds);
        o
    }

    /// Back to the three-quarter view, framed.
    pub fn reset(&mut self, bounds: &gz_render::Aabb) {
        let dir = START_OFFSET.normalize();
        self.pitch = dir.y.asin();
        self.yaw = dir.x.atan2(dir.z);
        self.frame(bounds);
    }

    /// The bounding sphere framed with a small margin, from wherever the camera looks now
    ///.
    pub fn frame(&mut self, bounds: &gz_render::Aabb) {
        self.pan = Vec3::ZERO;
        if bounds.is_empty() {
            self.center = Vec3::ZERO;
            self.radius = 1.0;
        } else {
            self.center = bounds.center();
            self.radius = (0.5 * bounds.size().length()).max(1e-3);
        }
        self.distance = self.radius / (FOV_DEGREES.to_radians() * 0.5).sin() * 1.12;
    }

    /// From the pivot towards the camera.
    #[must_use]
    pub fn dir(&self) -> Vec3 {
        Vec3::new(self.pitch.cos() * self.yaw.sin(), self.pitch.sin(), self.pitch.cos() * self.yaw.cos())
    }

    #[must_use]
    pub fn pivot(&self) -> Vec3 {
        self.center + self.pan
    }

    #[must_use]
    pub fn camera(&self) -> Camera {
        let pivot = self.pivot();
        let eye = pivot + self.dir() * self.distance;
        let near = (self.distance - self.radius * 2.0).max(0.01);
        let far = self.distance + self.radius * 4.0 + 1.0;
        Camera::look_at(eye, pivot).with_projection(Projection::Perspective {
            fov_y: FOV_DEGREES.to_radians(),
            near,
            far,
        })
    }

    /// The left button turns the model: `d` points dragged.
    pub fn orbit(&mut self, d: Vec2) {
        self.yaw += d.x * ORBIT_SPEED;
        self.pitch = (self.pitch + d.y * ORBIT_SPEED).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    }

    /// The right or middle button pans one to one: what a point covers at the pivot.
    pub fn pan_by(&mut self, d: Vec2, view_h: f32) {
        let per = 2.0 * self.distance * (FOV_DEGREES.to_radians() * 0.5).tan() / view_h.max(1.0);
        let fwd = -self.dir();
        let right = Vec3::Y.cross(fwd).try_normalize().unwrap_or(Vec3::X);
        let up = fwd.cross(right).try_normalize().unwrap_or(Vec3::Y);
        self.pan += right * (-d.x * per) + up * (d.y * per);
    }

    /// The wheel dollies, at the point under the cursor: what is under it stays under it.
    /// `at` is the cursor in the view's pixels, `size` the view's.
    pub fn zoom_at(&mut self, notches: f32, at: GVec2, size: [u32; 2]) {
        let before = self.distance;
        let after = (before * (-notches * 0.15).exp()).clamp(self.radius * 0.05, self.radius * 100.0);
        if after == before {
            return;
        }
        let cam = self.camera();
        let pivot = self.pivot();
        if let Some(ray) = pick_ray(&cam, size, at) {
            let n = cam.forward;
            let denom = ray.dir.dot(n);
            if denom.abs() > 1e-6 {
                let t = (pivot - ray.origin).dot(n) / denom;
                if t > 0.0 {
                    let hit = ray.origin + ray.dir * t;
                    self.pan += (hit - pivot) * (1.0 - after / before);
                }
            }
        }
        self.distance = after;
    }
}

/// 1, 2 or 5 times a power of ten, near `x`.
#[must_use]
pub fn round_step(x: f32) -> f32 {
    let p = 10f32.powf(x.max(1e-4).log10().floor());
    let m = x / p;
    p * if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    }
}

/// The floor the model stands on: lines at a round step in metres under the
/// lowest point of the box and a margin wider than it; every fifth stronger, the lines through
/// the origin the strongest. Each line with its tier: 0 minor, 1 every fifth, 2 the origin's.
#[must_use]
pub fn floor_lines(bounds: &gz_render::Aabb, radius: f32) -> Vec<(Vec3, Vec3, u8)> {
    if bounds.is_empty() {
        return Vec::new();
    }
    let margin = radius * FLOOR_MARGIN;
    let mut step = round_step(radius * 0.25);
    let (mut i0, mut i1, mut k0, mut k1);
    loop {
        i0 = ((bounds.min.x - margin) / step).floor() as i64;
        i1 = ((bounds.max.x + margin) / step).ceil() as i64;
        k0 = ((bounds.min.z - margin) / step).floor() as i64;
        k1 = ((bounds.max.z + margin) / step).ceil() as i64;
        if i1 - i0 < FLOOR_LINES_MAX && k1 - k0 < FLOOR_LINES_MAX {
            break;
        }
        step = round_step(step * 2.5);
    }
    let tier = |n: i64| {
        if n == 0 {
            2
        } else if n % 5 == 0 {
            1
        } else {
            0
        }
    };
    let y = bounds.min.y;
    let mut out = Vec::with_capacity(((i1 - i0 + 1) + (k1 - k0 + 1)) as usize);
    for i in i0..=i1 {
        out.push((
            Vec3::new(i as f32 * step, y, k0 as f32 * step),
            Vec3::new(i as f32 * step, y, k1 as f32 * step),
            tier(i),
        ));
    }
    for k in k0..=k1 {
        out.push((
            Vec3::new(i0 as f32 * step, y, k as f32 * step),
            Vec3::new(i1 as f32 * step, y, k as f32 * step),
            tier(k),
        ));
    }
    out
}

fn color_of(c: Color32) -> Color {
    Color::rgb(c.r(), c.g(), c.b())
}

/// What the viewport was last drawn for: a frame submits a new packet only when it changed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ViewKey {
    orbit: Orbit,
    size: [u32; 2],
    mode: ViewMode,
    bones: bool,
    model: u64,
}

/// The model tab.
pub struct ModelDoc {
    editor: Arc<Editor>,
    name: String,
    source: ModelSource,
    /// Where it was opened from: `project`, `library` or `game`.
    opened_from: String,
    load: Option<Pending<ModelData>>,
    renderer: Option<Pending<Result<Arc<Renderer>, String>>>,
    /// The load the renderer holds the mesh of.
    uploaded: u64,
    serial: u64,
    mesh: Option<MeshHandle>,
    pub orbit: Option<Orbit>,
    pub wireframe: bool,
    pub show_bones: bool,
    /// 0 Studio, 1 Unlit.
    pub lighting: usize,
    pub show_details: bool,
    /// pt, as the last frame laid it out.
    pub details_w: f32,
    drawn_frame: Option<u64>,
    last_key: Option<ViewKey>,
    packet: Arc<Mutex<Option<Arc<FramePacket>>>>,
    /// The renderer's target as egui draws it, by the target's serial.
    target: Option<(u64, Arc<ImageDraw>)>,
    gpu: Option<egui_wgpu::RenderState>,
    /// The serial of the last packet handed to the renderer.
    submitted: Arc<std::sync::atomic::AtomicU64>,
    /// The Reload Textures generation the model's textures were loaded at.
    reload_seen: u64,
}

impl ModelDoc {
    #[must_use]
    pub fn new(
        editor: &Arc<Editor>,
        name: String,
        source: ModelSource,
        load: Option<Pending<ModelData>>,
    ) -> Self {
        let renderer = editor.services.get::<WindowLink>().and_then(|l| l.gpu()).map(|gpu| {
            let (device, queue) = (gpu.device.clone(), gpu.queue.clone());
            let gate = gz_app::gpu_gate(editor);
            let exec = editor.exec.clone();
            let pools = exec.clone();
            Pending::spawn(&exec, async move {
                let inner = pools.clone();
                pools
                    .cpu(move || {
                        Renderer::new(device, queue, gate, inner).map(Arc::new).map_err(|e| e.to_string())
                    })
                    .await
            })
        });
        Self {
            editor: editor.clone(),
            name,
            source,
            opened_from: String::new(),
            load,
            renderer,
            uploaded: 0,
            serial: 1,
            mesh: None,
            orbit: None,
            wireframe: false,
            show_bones: false,
            lighting: 0,
            show_details: true,
            details_w: 0.0,
            drawn_frame: None,
            last_key: None,
            packet: Arc::new(Mutex::new(None)),
            target: None,
            gpu: None,
            submitted: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            reload_seen: gz_app::gpu::TextureReload::generation(editor),
        }
    }

    /// The model, once it is loaded.
    #[must_use]
    pub fn data(&self) -> Option<Arc<ModelData>> {
        self.load.as_ref().and_then(Pending::get)
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn source(&self) -> ModelSource {
        self.source
    }

    /// The load in flight, for a command to wait on.
    #[must_use]
    pub fn pending(&self) -> Option<Pending<ModelData>> {
        self.load.clone()
    }

    /// The renderer, once it exists.
    #[must_use]
    pub fn renderer(&self) -> Option<Arc<Renderer>> {
        self.renderer.as_ref().and_then(Pending::get).and_then(|r| r.as_ref().as_ref().ok().cloned())
    }

    /// Drawn in the frame being built or the one before it.
    #[must_use]
    pub fn drawn_lately(&self, frame: u64) -> bool {
        self.drawn_frame.is_some_and(|f| f + 1 >= frame)
    }

    fn shown(&self) -> Option<Arc<ModelData>> {
        self.data().filter(|d| d.shown())
    }

    fn bounds(&self) -> gz_render::Aabb {
        self.data().map_or(gz_render::Aabb::EMPTY, |d| d.bounds)
    }

    pub fn frame_model(&mut self) {
        let b = self.bounds();
        if let Some(o) = self.orbit.as_mut() {
            o.frame(&b);
        }
    }

    pub fn reset_view(&mut self) {
        let b = self.bounds();
        if let Some(o) = self.orbit.as_mut() {
            o.reset(&b);
        }
    }

    /// The view asked for over MCP; `None` keeps.
    pub fn apply_view(
        &mut self,
        lighting: Option<usize>,
        wireframe: Option<bool>,
        bones: Option<bool>,
        details: Option<bool>,
        frame: bool,
        reset: bool,
    ) {
        if let Some(l) = lighting {
            self.lighting = l.min(1);
        }
        if let Some(w) = wireframe {
            self.wireframe = w;
        }
        if let Some(b) = bones {
            self.show_bones = b;
        }
        if let Some(d) = details {
            self.show_details = d;
        }
        if reset {
            self.reset_view();
        } else if frame {
            self.frame_model();
        }
    }

    fn mode(&self) -> ViewMode {
        if self.wireframe {
            ViewMode::Wireframe
        } else if self.lighting == 1 {
            ViewMode::Unlit
        } else {
            ViewMode::Lit
        }
    }

    /// What the tab draws: the model, the floor under it, the bones when asked.
    #[must_use]
    pub fn draw_list(&self, ctx: &egui::Context) -> Option<DrawList> {
        let data = self.shown()?;
        let mesh = self.mesh?;
        let orbit = self.orbit?;
        let p = palette(ctx);
        let mut list = DrawList::new();
        let mut inst =
            Instance::new(mesh, Mat4::IDENTITY).with_flags(InstanceFlags::NO_ID | InstanceFlags::NO_CULL);
        inst.bounds = Some(data.bounds);
        if let Some(pose) = &data.pose {
            inst = inst.with_pose(pose.clone());
        }
        list.push(inst);
        // the floor goes down under the model, which then covers what it stands on
        let tiers = [color_of(p.border_subtle), color_of(p.border), color_of(p.border_strong)];
        for (a, b, t) in floor_lines(&data.bounds, orbit.radius) {
            list.helpers.line(a, b, tiers.get(usize::from(t)).copied().unwrap_or(tiers[0]), true);
        }
        if self.show_bones && data.bones > 0 {
            // scaled to the model, or the markers vanish or swallow it
            let js = orbit.radius * JOINT_SIZE * 2.0;
            for (at, parent) in &data.joints {
                list.helpers.cross(*at, js, false, Color::from_argb(JOINT_COLOR), false);
                if let Some(up) = parent {
                    list.helpers.line(*up, *at, Color::from_argb(LINK_COLOR), false);
                }
            }
        }
        Some(list)
    }

    fn view_for(&self, ctx: &egui::Context, size: [u32; 2]) -> Option<View> {
        let orbit = self.orbit?;
        let p = palette(ctx);
        let mut view = View::new(orbit.camera(), size);
        view.mode = self.mode();
        view.flags = ViewFlags::HELPERS;
        view.background = color_of(p.canvas);
        Some(view)
    }

    /// Takes the loaded model into the renderer once.
    fn take_model(&mut self, renderer: &Arc<Renderer>) {
        let Some(data) = self.data() else { return };
        // Libraries > Reload Textures: the same sources again drop what was loaded
        let reload = gz_app::gpu::TextureReload::generation(&self.editor);
        if self.uploaded == self.serial && reload != self.reload_seen {
            self.reload_seen = reload;
            if let Some(sources) = &data.sources {
                renderer.assets().set_loader(Some(Arc::new(sources.clone())));
                self.last_key = None;
            }
            return;
        }
        if self.uploaded == self.serial {
            return;
        }
        self.reload_seen = reload;
        self.uploaded = self.serial;
        self.orbit = Some(Orbit::new(&data.bounds));
        self.last_key = None;
        if let (Some(mesh), Some(sources)) = (&data.mesh, &data.sources) {
            renderer.assets().set_loader(Some(Arc::new(sources.clone())));
            self.mesh = Some(renderer.assets().add_mesh(MESH_NAME, mesh.clone()));
        } else {
            self.mesh = None;
        }
    }

    fn toolbar_items<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>) {
        let Some(data) = self.shown() else { return };
        tb.group(3);
        if tb.action(
            "frame",
            Icon::FrameAll,
            "Frame",
            "Frame the model, keeping the angle (F)",
            true,
            false,
            Kind::Ghost,
        ) {
            self.frame_model();
        }
        if tb.action(
            "reset_view",
            Icon::Refresh,
            "Reset View",
            "Back to the three-quarter view, framed",
            true,
            false,
            Kind::Ghost,
        ) {
            self.reset_view();
        }
        tb.group(2);
        if tb.action(
            "wireframe",
            Icon::Wireframe,
            "Wireframe",
            "Draw the triangles' edges only",
            true,
            self.wireframe,
            Kind::Ghost,
        ) {
            self.wireframe = !self.wireframe;
        }
        let bones = data.bones > 0 && data.object.is_none();
        let tip = if bones { "Draw the skeleton over the model" } else { "The model has no bones" };
        if tb.action("bones", Icon::Bones, "Bones", tip, bones, self.show_bones && bones, Kind::Ghost) {
            self.show_bones = !self.show_bones;
        }
        tb.group(5);
        tb.segmented("lighting", "Lighting", &mut self.lighting, &["Studio", "Unlit"], "Studio lights the model from the camera, over a soft ambient; Unlit shows its textures as they are", true);
        tb.group(4);
        if tb.action(
            "details",
            Icon::SidebarRight,
            "Details",
            "Show what the model is made of beside the viewport",
            true,
            self.show_details,
            Kind::Ghost,
        ) {
            self.show_details = !self.show_details;
        }
        tb.separator();
        tb.group(1);
        let reference = self.object_ref(&data);
        let place_tip = if reference.is_some() {
            "Place it in the level, on the surface in front of the camera"
        } else {
            "Only a library object (.object) is placed in a level"
        };
        if tb.action(
            "place",
            Icon::Place,
            "Place in Level",
            place_tip,
            reference.is_some(),
            false,
            Kind::Ghost,
        ) && let Some(r) = &reference
        {
            self.editor.post(
                "scene.place",
                gz_app::args(&[("kind", "object"), ("ref", r.as_str()), ("on_surface", "true")]),
            );
        }
        let reveal = self.reveal_args(&data);
        let reveal_tip = if reveal.is_some() {
            "Select it in the Content Browser"
        } else {
            "The Content Browser lists the project's files and the Library's objects"
        };
        if tb.action(
            "reveal",
            Icon::Reveal,
            "Show in Content Browser",
            reveal_tip,
            reveal.is_some(),
            false,
            Kind::Ghost,
        ) && let Some(args) = reveal
        {
            self.editor.post("content.view", args);
        }
    }

    /// The library reference of an object on show (`props\box`), as `scene.place` takes it.
    fn object_ref(&self, data: &ModelData) -> Option<String> {
        data.object.as_ref()?;
        let name = crate::paths::strip_ext(&self.name);
        let lower = name.to_ascii_lowercase();
        Some(if lower.starts_with("rawdata\\objects\\") { name[16..].to_owned() } else { name.to_owned() })
    }

    /// What `content.view` reveals the model by: an object of the Library or the project.
    fn reveal_args(&self, data: &ModelData) -> Option<gz_editor::Args> {
        let r = self.object_ref(data)?;
        let in_project = self.opened_from == "project";
        let path = format!("{r}.object");
        Some(if in_project {
            gz_app::args(&[
                ("action", "reveal"),
                ("source", "project"),
                ("path", &format!("rawdata\\objects\\{path}")),
            ])
        } else {
            gz_app::args(&[("action", "reveal"), ("source", "library"), ("kind", "objects"), ("path", &path)])
        })
    }

    fn draw_viewport(&mut self, ui: &mut Ui, rect: Rect) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.canvas);
        let resp = ui.interact(rect, ui.scope_id().with("model.canvas"), Sense::click_and_drag());
        let ppp = ctx.pixels_per_point();
        let size =
            [((rect.width() * ppp).round() as u32).max(1), ((rect.height() * ppp).round() as u32).max(1)];
        if let Some(o) = self.orbit.as_mut() {
            // navigation goes first: the frame rendered next already shows it
            let d = resp.drag_delta();
            if resp.dragged_by(PointerButton::Secondary) || resp.dragged_by(PointerButton::Middle) {
                o.pan_by(d, rect.height());
            } else if resp.dragged_by(PointerButton::Primary) {
                o.orbit(d);
            }
            if resp.hovered() {
                let wheel = crate::local::local_wheel(ui);
                if wheel != 0.0
                    && let Some(at) = ctx.pointer_hover_pos()
                {
                    let px = (at - rect.min) * ppp;
                    o.zoom_at(wheel, GVec2::new(px.x, px.y), size);
                }
            }
        }
        if resp.double_clicked() {
            self.frame_model();
        }

        let renderer = self.renderer();
        let Some(renderer) = renderer else {
            let failed = self
                .renderer
                .as_ref()
                .and_then(Pending::get)
                .and_then(|r| r.as_ref().as_ref().err().cloned());
            let mut child =
                ui.new_child(UiBuilder::new().max_rect(rect).layout(Layout::top_down(Align::Center)));
            match failed {
                Some(e) => {
                    state::empty_state(&mut child, Icon::Warning, "The viewport cannot be drawn", &e, None);
                }
                None if self.renderer.is_none() => {
                    state::empty_state(
                        &mut child,
                        Icon::Warning,
                        "The viewport cannot be drawn",
                        "The editor runs without a graphics device.",
                        None,
                    );
                }
                None => state::busy_line(&mut child, "Starting the viewport..."),
            }
            return;
        };
        self.take_model(&renderer);

        // a new packet only when what it shows changed, or the assets it waited on arrived
        if let (Some(list), Some(view)) = (self.draw_list(&ctx), self.view_for(&ctx, size)) {
            let key = ViewKey {
                orbit: self.orbit.unwrap_or_else(|| Orbit::new(&gz_render::Aabb::EMPTY)),
                size,
                mode: self.mode(),
                bones: self.show_bones,
                model: self.uploaded,
            };
            let stale = self.packet.lock().as_ref().is_some_and(|pk| pk.is_stale(renderer.assets()));
            if self.last_key != Some(key) || stale {
                self.last_key = Some(key);
                let slot = self.packet.clone();
                let r = renderer.clone();
                let submitted = self.submitted.clone();
                self.editor.exec.spawn_cpu(move || {
                    let packet = Arc::new(build_packet(&list, &view, r.assets()));
                    let serial = r.submit(packet.clone());
                    *slot.lock() = Some(packet);
                    submitted.fetch_max(serial, std::sync::atomic::Ordering::AcqRel);
                });
            }
        }

        if let (Some(handle), Some(gpu)) = (renderer.target(), self.gpu.as_ref()) {
            if self.target.as_ref().is_none_or(|(serial, _)| *serial != handle.serial)
                && let Some(draw) = ImageDraw::new(gpu, &handle.view, true, self.editor.exec.wake())
            {
                self.target = Some((handle.serial, draw));
            }
            if let Some((_, draw)) = &self.target {
                let [u, v] = handle.uv();
                // the view is drawn opaque: the target's alpha is a texture's gloss mask, not coverage
                paint_image(ui.painter(), rect, rect, [0.0, 0.0, u, v], ImageMode::Opaque, draw);
            }
        }
    }

    fn draw_details(&self, ui: &mut Ui) {
        let Some(data) = self.shown() else {
            state::empty_state(ui, Icon::Info, "No facts", "Open a model to see what it is made of.", None);
            return;
        };
        let b = data.bounds;
        let (size, centre) = if b.is_empty() { (Vec3::ZERO, Vec3::ZERO) } else { (b.size(), b.center()) };
        let radius = self.orbit.map_or(0.0, |o| o.radius);
        let ro = PropFlags::READ_ONLY;
        PropertyGrid::new("model").show(ui, |g| {
            g.section("Asset", true, None, |g| {
                g.row("Name", "The model's file name", ro, 0, |ui| read_only(ui, crate::paths::leaf(&data.name), Clip::End));
                g.row("Path", "Where it lives, below the game's meshes or the project", ro, 0, |ui| read_only(ui, &data.name, Clip::Start));
                g.row("Kind", "What kind of visual the engine sees", ro, 0, |ui| read_only(ui, &data.kind, Clip::End));
            });
            g.section("Geometry", true, None, |g| {
                g.row("Vertices", "", ro, 0, |ui| read_only(ui, &data.verts.to_string(), Clip::End));
                g.row("Triangles", "", ro, 0, |ui| read_only(ui, &data.tris.to_string(), Clip::End));
                if data.bones > 0 {
                    g.row("Bones", "", ro, 0, |ui| read_only(ui, &data.bones.to_string(), Clip::End));
                }
                if let Some(o) = &data.object {
                    g.row("Meshes", "", ro, 0, |ui| read_only(ui, &o.meshes.to_string(), Clip::End));
                    if !o.lod.is_empty() {
                        g.row("LOD", "The level-of-detail model it switches to", ro, 0, |ui| read_only(ui, &o.lod, Clip::Start));
                    }
                    if o.skeleton {
                        g.row("Motions", "", ro, 0, |ui| read_only(ui, &o.motions.to_string(), Clip::End));
                    }
                }
            });
            g.section("Bounds", true, None, |g| {
                g.row("Size", "Its box, in metres", ro, 0, |ui| read_only(ui, &format!("{:.2} x {:.2} x {:.2}", size.x, size.y, size.z), Clip::End));
                g.row("Centre", "The box's centre", ro, 0, |ui| read_only(ui, &format!("{:.2}, {:.2}, {:.2}", centre.x, centre.y, centre.z), Clip::End));
                g.row("Radius", "The radius of the sphere around it", ro, 0, |ui| read_only(ui, &format!("{radius:.2}"), Clip::End));
            });
            // what the engine visual draws with, and where each texture comes from: one the loader
            // cannot find draws as its placeholder
            if !data.textures.is_empty() {
                let badge = data.textures.len().to_string();
                g.section("Textures", true, Some(&badge), |g| {
                    for t in &data.textures {
                        let (word, why) = match t.from {
                            "project" => ("Project", "The project's own file"),
                            "editor" => ("Library", "The editor's library (Preferences > Library)"),
                            "game" => ("Game", "The linked game's archives or its gamedata folder"),
                            _ => ("Not found", "Neither the editor nor the linked game has it: the preview draws the placeholder"),
                        };
                        let hint = format!("{}\n{why}", t.name);
                        g.row(crate::paths::leaf(&t.name), &hint, ro, 0, |ui| {
                            if t.from == "missing" {
                                let p = palette(ui.ctx());
                                text::label(ui, word, Font::Body, p.warning, Clip::End);
                            } else {
                                read_only(ui, word, Clip::End);
                            }
                        });
                    }
                });
            }
            // the material slots of a library object: texture, shader, game material
            if let Some(o) = data.object.as_ref().filter(|o| !o.materials.is_empty()) {
                let badge = o.materials.len().to_string();
                g.section("Materials", true, Some(&badge), |g| {
                    for m in &o.materials {
                        let dash = |s: &str| if s.is_empty() { "-".to_owned() } else { s.to_owned() };
                        g.row(if m.name.is_empty() { "(unnamed)" } else { &m.name }, "The texture of this slot", ro, 0, |ui| read_only(ui, &dash(&m.texture), Clip::Start));
                        g.row("Shader", "The engine shader it draws with", ro, 1, |ui| read_only(ui, &dash(&m.shader), Clip::End));
                        g.row("Game Material", "What the game makes of hits and steps on it", ro, 1, |ui| read_only(ui, &dash(&m.game_mtl), Clip::End));
                    }
                });
            }
        });
    }

    /// `preview.model`'s fields after `tab` and `active`.
    #[must_use]
    pub fn state_json(&self) -> serde_json::Map<String, Value> {
        let mut out = serde_json::Map::new();
        let data = self.data();
        let shown = data.as_ref().is_some_and(|d| d.shown());
        let d = data.as_deref();
        out.insert("open".into(), json!(true));
        out.insert("name".into(), json!(self.name));
        out.insert("error".into(), json!(d.map_or("", |d| d.error.as_str())));
        out.insert("kind".into(), json!(d.filter(|_| shown).map_or("", |d| d.kind.as_str())));
        out.insert("verts".into(), json!(d.map_or(0, |d| d.verts)));
        out.insert("tris".into(), json!(d.map_or(0, |d| d.tris)));
        let bones = d.map_or(0, |d| d.bones);
        out.insert("bones".into(), json!(bones));
        out.insert("lighting".into(), json!(if self.lighting == 1 { "unlit" } else { "studio" }));
        out.insert("wireframe".into(), json!(self.wireframe));
        let visual = d.is_some_and(|d| d.object.is_none());
        out.insert("bones_shown".into(), json!(self.show_bones && bones > 0 && visual && shown));
        out.insert("panes".into(), json!({ "details": self.show_details && shown, "details_width": if self.show_details { self.details_w.round() } else { 0.0 } }));
        let textures: Vec<Value> = d.map_or_else(Vec::new, |d| {
            d.textures
                .iter()
                .map(|t| json!({ "name": crate::paths::json_path_str(&t.name), "from": t.from }))
                .collect()
        });
        out.insert("textures".into(), Value::Array(textures));
        out
    }

    /// The next frame hands the renderer a new packet, built with what has arrived since;
    /// answers the serial of the last one handed over before it.
    pub fn redraw(&mut self) -> u64 {
        self.last_key = None;
        self.submitted()
    }

    /// The serial of the last packet handed to the renderer.
    #[must_use]
    pub fn submitted(&self) -> u64 {
        self.submitted.load(std::sync::atomic::Ordering::Acquire)
    }
}

impl Document for ModelDoc {
    fn id(&self) -> &str {
        "model"
    }
    fn kind(&self) -> &'static str {
        "model"
    }
    fn label(&self) -> String {
        if self.name.is_empty() { "Model Preview".to_owned() } else { crate::paths::label_of(&self.name) }
    }
    fn icon(&self) -> Icon {
        Icon::FileModel
    }
    fn path(&self) -> Option<String> {
        (!self.name.is_empty()).then(|| self.name.clone())
    }
    fn tooltip(&self) -> Option<String> {
        Some(if self.name.is_empty() { "Model Preview".to_owned() } else { self.name.clone() })
    }
    fn padded(&self) -> bool {
        false
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, _cx: &mut DocCx<'_>) {
        self.toolbar_items(tb);
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let ctx = ui.ctx().clone();
        let frame = ctx.cumulative_frame_nr();
        self.drawn_frame = Some(frame);
        if self.gpu.is_none() {
            self.gpu = Some(cx.gpu.clone());
            // the picture's pass is made while the picture loads
            let _ = crate::image_pass::ImagePass::get(cx.gpu, cx.editor.exec.wake());
        }
        let full = ui.max_rect();
        let data = self.data();
        let shown = data.as_ref().is_some_and(|d| d.shown());
        let with_details = self.show_details && shown;
        let (canvas, details) = if with_details {
            let split = splitter_tail(ui, "model.details", true, CANVAS_MIN, SIDE_MIN, DETAILS_W);
            self.details_w = split.size;
            let gap = ui.spacing().item_spacing.x;
            let canvas = Rect::from_min_max(
                full.min,
                pos2((full.max.x - split.size - gap).max(full.min.x + 1.0), full.max.y),
            );
            (canvas, Some(Rect::from_min_max(pos2(full.max.x - split.size, full.min.y), full.max)))
        } else {
            self.details_w = 0.0;
            (full, None)
        };

        match data {
            None if self.load.is_some() => {
                let p = palette(&ctx);
                ui.painter().rect_filled(canvas, CornerRadius::ZERO, p.canvas);
                let mut child =
                    ui.new_child(UiBuilder::new().max_rect(canvas).layout(Layout::top_down(Align::Center)));
                state::busy_line(&mut child, &format!("Loading {}...", crate::paths::leaf(&self.name)));
            }
            None => {
                let p = palette(&ctx);
                ui.painter().rect_filled(canvas, CornerRadius::ZERO, p.canvas);
                let mut child =
                    ui.new_child(UiBuilder::new().max_rect(canvas).layout(Layout::top_down(Align::Center)));
                state::empty_state(
                    &mut child,
                    Icon::FileModel,
                    "No model",
                    "Double-click a model in the Content Browser to see it here.",
                    None,
                );
            }
            Some(d) if !d.error.is_empty() => {
                let p = palette(&ctx);
                ui.painter().rect_filled(canvas, CornerRadius::ZERO, p.canvas);
                let mut child =
                    ui.new_child(UiBuilder::new().max_rect(canvas).layout(Layout::top_down(Align::Center)));
                state::empty_state(
                    &mut child,
                    Icon::Warning,
                    "This model cannot be shown",
                    &format!("{}: {}", d.name, d.error),
                    None,
                );
            }
            Some(_) => self.draw_viewport(ui, canvas),
        }

        if let Some(rect) = details {
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt("model.details.pane")
                    .max_rect(rect.shrink(8.0))
                    .layout(Layout::top_down(Align::Min)),
            );
            ScrollArea::vertical()
                .id_salt("model.details.scroll")
                .auto_shrink([false, false])
                .show(&mut child, |ui| self.draw_details(ui));
        }
        ui.allocate_rect(full, Sense::hover());
    }

    fn status(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        let Some(d) = self.shown() else { return };
        let mut text_ = format!("{} verts \u{b7} {} tris", d.verts, d.tris);
        if d.bones > 0 {
            text_.push_str(&format!(" \u{b7} {} bones", d.bones));
        }
        text_.push_str(if self.lighting == 1 { " \u{b7} Unlit" } else { " \u{b7} Studio" });
        let p = palette(ui.ctx());
        text::label(ui, &text_, Font::Small, p.text_secondary, Clip::End);
    }

    fn shortcut(&mut self, _cx: &mut DocCx<'_>, chord: &Chord) -> bool {
        if chord.key == egui::Key::F && !chord.ctrl && !chord.shift && !chord.alt && self.shown().is_some() {
            self.frame_model();
            return true;
        }
        false
    }

    fn on_close(&mut self, _cx: &mut DocCx<'_>) {
        self.load = None;
        self.mesh = None;
    }

    fn restore_arg(&self) -> Option<String> {
        (!self.name.is_empty()).then(|| self.name.clone())
    }

    fn mcp_state(&self) -> Value {
        Value::Object(self.state_json())
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// The model a command loaded before it opened the tab: the factory takes it instead of loading
/// the same file again.
#[derive(Default)]
pub struct ModelPreload {
    slot: Mutex<Option<(String, ModelSource, Pending<ModelData>)>>,
}

impl ModelPreload {
    pub fn put(&self, name: &str, source: ModelSource, load: Pending<ModelData>) {
        *self.slot.lock() = Some((name.to_owned(), source, load));
    }

    fn take(&self, name: &str, source: ModelSource) -> Option<Pending<ModelData>> {
        let mut s = self.slot.lock();
        match s.take() {
            Some((n, src, p)) if n == name && src == source => Some(p),
            other => {
                *s = other;
                None
            }
        }
    }
}

/// The `model` factory: `path` is the model's name, `arg` its source (`visual` | `darf`).
pub fn factory(editor: &Arc<Editor>, req: &OpenRequest) -> Result<Box<dyn Document>, String> {
    let name = crate::paths::engine_name(req.path.as_deref().unwrap_or(""));
    let source = ModelSource::parse(req.arg.as_deref().unwrap_or(""));
    if name.is_empty() {
        return Ok(Box::new(ModelDoc::new(editor, String::new(), source, None)));
    }
    let preload = editor.services.get_or_init(ModelPreload::default);
    let load = preload.take(&name, source).unwrap_or_else(|| {
        let (ed, n) = (editor.clone(), name.clone());
        Pending::spawn(&editor.exec, load(ed, n, source))
    });
    let mut doc = ModelDoc::new(editor, name, source, Some(load));
    doc.opened_from = req.arg.clone().unwrap_or_default();
    Ok(Box::new(doc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_round() {
        assert!((round_step(0.3) - 0.2).abs() < 1e-6);
        assert!((round_step(1.2) - 1.0).abs() < 1e-6);
        assert!((round_step(4.0) - 5.0).abs() < 1e-6);
        assert!((round_step(8.0) - 10.0).abs() < 1e-5);
    }

    #[test]
    fn the_floor_is_under_the_model_and_bounded() {
        let b = gz_render::Aabb::new(Vec3::new(-1.0, 0.5, -1.0), Vec3::new(1.0, 2.0, 1.0));
        let lines = floor_lines(&b, 1.5);
        assert!(!lines.is_empty());
        assert!(lines.iter().all(|(a, c, _)| a.y == 0.5 && c.y == 0.5));
        assert!(lines.iter().any(|(_, _, t)| *t == 2));
        let huge = gz_render::Aabb::new(Vec3::splat(-1000.0), Vec3::splat(1000.0));
        assert!(floor_lines(&huge, 1.0).len() <= 2 * FLOOR_LINES_MAX as usize + 2);
    }

    #[test]
    fn the_orbit_frames_and_keeps_its_limits() {
        let b = gz_render::Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        let mut o = Orbit::new(&b);
        assert!(o.pitch > 0.0 && o.distance > o.radius);
        o.orbit(Vec2::new(0.0, 10_000.0));
        assert!((o.pitch - PITCH_LIMIT).abs() < 1e-6);
        let d = o.distance;
        o.zoom_at(1.0, GVec2::new(100.0, 100.0), [200, 200]);
        assert!(o.distance < d);
        o.zoom_at(-1000.0, GVec2::new(100.0, 100.0), [200, 200]);
        assert!((o.distance - o.radius * 100.0).abs() < 1e-3);
        let cam = o.camera();
        assert!((cam.position - (o.pivot() + o.dir() * o.distance)).length() < 1e-3);
    }
}
