//! [`ViewportPane`]: the picture, the pills, the gizmo, the drop target and the mouse.
//!
//! What a left press does follows the scene's mode, read from the scene's snapshot each press:
//! Select picks, draws a marquee with Ctrl and drags the gizmo; Place puts down what is armed
//! at each click; Paint paints the layer its panel picked (Ctrl picks and boxes the layer's own
//! things); Zones draws the visibility cells. A plain left drag that no mode takes flies the
//! camera, and a click selects on release, under 4 points of movement - the camera never takes
//! a click from a mode.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use egui::{
    Align, Align2, Color32, CornerRadius, Key, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder,
    Vec2, pos2, vec2,
};
use glam::{Vec2 as G2, Vec3};
use gz_app::{Chord, DocCx};
use gz_editor::{Editor, SmolStr, Value, json};
use gz_render::{Color, Ray};
use gz_scene::apply::{SelectMode, TransformEdit};
use gz_scene::objtools::ToolEdit;
use gz_scene::{ObjClass, ObjData, ObjId, Object, SceneCmd, SceneMode, SceneService};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Kind, Palette, TipExt, Toolbar, ToolbarStyle, Tx, palette, tr};
use parking_lot::Mutex;

use crate::camera::{Buttons, FlyKey, Target};
use crate::gizmo::{self, Anchor, GizmoType, Shapes, Space, Status};
use crate::menus;
use crate::place::Placeable;
use crate::service::Viewport;

/// One gesture of the mouse in the viewport.
enum Gesture {
    None,
    /// The camera took the press.
    Camera,
    /// A left press no mode took at once: a drag that follows flies the camera, a Ctrl drag
    /// draws a marquee, and a click - the button up before the pointer went `DRAG_START` from
    /// `start` - does what the press waits to do.
    Press {
        start: Pos2,
        click: Option<Click>,
    },
    Transform(Box<Drag>),
    Marquee {
        start: Pos2,
        now: Pos2,
    },
    /// Zones: meshes painted into the selected cell.
    ToolPaint(crate::objtools::Paint),
    /// Zones: a box of meshes for the selected cell.
    ToolBox {
        start: Pos2,
        now: Pos2,
    },
    /// Paint > AI Map: a stroke of the brush.
    AiPaint(crate::bulktools::Paint),
    /// Paint > Foliage: the brush's spots on the ground, painted (or erased) on release.
    Foliage(Box<BrushStroke>),
}

/// What a left click does once the button is up, kept from the press: a drag in between does
/// none of it.
enum Click {
    /// Select what the ray hits, with the keys of the press.
    Select { ray: Ray, b: Buttons },
    /// A painted layer's own pick (its nodes, slots or marks).
    Layer { ray: Ray, b: Buttons, layer: ObjClass },
    /// Place the armed thing where the ray lands.
    Place { ray: Ray },
    /// Paint > Decals: lay a wallmark (Shift moves the selected one there).
    Decal { ray: Ray, b: Buttons },
}

/// How far the pointer may go from a left press before the press is a drag: a click below it, the
/// camera or the marquee past it (points).
const DRAG_START: f32 = 4.0;

/// What a left press that waits for its click turns into this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PressStep {
    /// Still a click, if the button comes up now.
    Wait,
    /// The button came up: the click.
    Click,
    /// The button came up after a drag no one took: nothing.
    Nothing,
    /// Ctrl and a drag: the marquee.
    Marquee,
    /// A drag with no key: the camera.
    Camera,
    /// A drag with another key: no click any more, nothing else either.
    Drag,
}

fn press_step(b: &Buttons, start: Pos2, now: Pos2, clicks: bool) -> PressStep {
    let dragged = (now - start).length() > DRAG_START;
    if !b.left {
        return if clicks && !dragged { PressStep::Click } else { PressStep::Nothing };
    }
    if !dragged {
        return PressStep::Wait;
    }
    if b.ctrl {
        PressStep::Marquee
    } else if !(b.right || b.middle || b.alt || b.shift) {
        PressStep::Camera
    } else {
        PressStep::Drag
    }
}

/// What the camera turns by this frame while it holds the cursor at `hold`, in pixels: the mouse's
/// own motion when the window has it (raw input, endless at any screen edge), else how far the cursor
/// went from where it is held back to every frame.
fn held_delta(motion: Option<Vec2>, pos: Option<Pos2>, hold: Pos2, ppp: f32) -> Vec2 {
    motion.unwrap_or_else(|| pos.map_or(Vec2::ZERO, |p| (p - hold) * ppp))
}

/// An object a transform drag holds, as it was at the press.
#[derive(Clone, Copy, Debug)]
struct Held {
    id: ObjId,
    position: Vec3,
    rotation: Vec3,
    scale: Vec3,
    glow: bool,
    /// A dynamic library object keeps its scale.
    dynamic: bool,
}

/// A drag of a gizmo handle.
struct Drag {
    kind: GizmoType,
    status: Status,
    ray0: Ray,
    /// Where the press was, points from the pane's corner.
    press: Pos2,
    /// The real pivot and the gizmo's axes at the press.
    pivot: Vec3,
    axes: [Vec3; 3],
    space: Space,
    /// The handles on the screen at the press (a scale drag and an edge-on ring measure on them).
    shapes: Shapes,
    /// A ring seen edge-on: its angle is the pointer's travel across it.
    edge: bool,
    held: Vec<Held>,
    /// Way paths with selected points: those points, as at the press.
    points: Vec<(ObjId, Vec<(usize, Vec3)>)>,
    /// The AI map's selected nodes, as at the press.
    nodes: Vec<(u32, Vec3)>,
    /// The turn so far (radians), for the arc and its readout; and the part of it the AI map's
    /// nodes were already given.
    angle: f32,
    applied: f32,
    merge: SmolStr,
    svc: Arc<SceneService>,
    /// Alt: the copies the drag moves, once they are made.
    copies: Option<Arc<Mutex<Option<Vec<ObjId>>>>>,
}

/// A stroke of the Foliage brush: its spots on the ground, found as the pointer moves.
struct BrushStroke {
    erase: bool,
    spots: Arc<Mutex<Vec<Vec3>>>,
    picking: Arc<AtomicUsize>,
    last: Pos2,
}

/// Where the ground is under the pointer, asked one pick at a time: the ghost of a drop and the
/// brush's ring stand on it.
#[derive(Default)]
struct Ground {
    busy: Arc<AtomicBool>,
    asked: Option<Pos2>,
    hit: Arc<Mutex<Option<(Vec3, Vec3)>>>,
}

impl Ground {
    fn track(&mut self, editor: &Arc<Editor>, at: Pos2, ray: Ray) {
        if self.asked == Some(at) || self.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        self.asked = Some(at);
        let Ok(svc) = SceneService::of(editor) else {
            self.busy.store(false, Ordering::Release);
            return;
        };
        let (ed, busy, hit) = (editor.clone(), self.busy.clone(), self.hit.clone());
        editor.exec.spawn(async move {
            let found = crate::place::landing(&ed, &svc, ray).await;
            *hit.lock() = found.map(|(p, up)| (p, up.unwrap_or(Vec3::Y)));
            busy.store(false, Ordering::Release);
            ed.exec.wake().ping();
        });
    }

    fn at(&self) -> Option<(Vec3, Vec3)> {
        *self.hit.lock()
    }

    fn forget(&mut self) {
        self.asked = None;
        *self.hit.lock() = None;
    }
}

/// The 3D view of the Level tab. UI thread only.
pub struct ViewportPane {
    editor: Arc<Editor>,
    vp: Arc<Viewport>,
    texture: Option<(egui::TextureId, u64)>,
    gesture: Gesture,
    hover: Status,
    serial: u64,
    /// The canvas as last laid out, points.
    rect: Rect,
    /// Where the gesture fed over MCP had the pointer last frame.
    fed_at: Option<Pos2>,
    /// Where the camera holds the hidden cursor while it has the mouse (the press), points.
    hold: Option<Pos2>,
    ground: Ground,
    /// The pointer over the canvas this frame (points, absolute), and the ray through it.
    pointer: Option<(Pos2, Ray)>,
}

impl ViewportPane {
    #[must_use]
    pub fn new(editor: &Arc<Editor>, gpu: &egui_wgpu::RenderState) -> Self {
        let vp = Viewport::of(editor);
        vp.start_renderer(editor, gpu);
        crate::bulktools::provide_axes(editor);
        Self {
            editor: editor.clone(),
            vp,
            texture: None,
            gesture: Gesture::None,
            hover: Status::None,
            serial: 0,
            rect: Rect::NOTHING,
            fed_at: None,
            hold: None,
            ground: Ground::default(),
            pointer: None,
        }
    }

    /// Draws the viewport into the space it is given: the picture, the pills, the gizmo, input.
    pub fn ui(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let (rect, canvas) =
            ui.allocate_exact_size(ui.available_size().max(vec2(32.0, 32.0)), Sense::click_and_drag());
        self.rect = rect;
        let ppp = ctx.pixels_per_point();
        self.vp.set_pane([rect.width(), rect.height()], ppp);
        let size =
            [(rect.width() * ppp).round().max(32.0) as u32, (rect.height() * ppp).round().max(32.0) as u32];
        let renderer = self.vp.start_renderer(cx.editor, cx.gpu);
        if renderer.is_none() && self.vp.renderer_starting() {
            // the renderer starts on a thread of its own: look again soon
            ctx.request_repaint_after(Duration::from_millis(16));
        }
        let canvas_c = Color::rgb(p.canvas.r(), p.canvas.g(), p.canvas.b());
        let lines = [p.scene_grid, p.scene_grid_major, p.scene_wire].map(|c| Color::rgb(c.r(), c.g(), c.b()));
        if self.vp.read(|s| s.size != size || s.background != canvas_c || s.scene_lines != lines) {
            self.vp.with(|s| {
                s.size = size;
                s.background = canvas_c;
                s.scene_lines = lines;
            });
        }
        self.vp.sync(cx.editor);

        self.input(&ctx, &canvas, rect, ppp, size);

        self.vp.request_build(cx.editor);
        let painter = ui.painter_at(rect);
        let game = self.vp.read(|s| {
            s.game.active().then(|| crate::lighting::clear_color(&s.game.at, s.game.exposure_now()))
        });
        let bg = game.map_or(p.canvas, |c| Color32::from_rgb(c.0[0], c.0[1], c.0[2]));
        painter.rect_filled(rect, CornerRadius::ZERO, bg);
        let mut finished = None;
        let mut label_px = ppp * self.vp.read(|s| s.prefs.render_scale);
        if let Some(r) = &renderer {
            let info = r.finished();
            self.vp.rates.lock().tick(info.frame);
            if let Some(t) = r.target() {
                let id = self.texture_of(cx.gpu, &t);
                let [u, v] = t.uv();
                label_px = t.size[0] as f32 / rect.width().max(1.0);
                painter.image(id, rect, Rect::from_min_max(Pos2::ZERO, pos2(u, v)), Color32::WHITE);
            }
            finished = Some(info);
        }
        if let Some(info) = &finished {
            draw_labels(&painter, rect, info, label_px, p.scene_label_shadow);
        }
        let show = self.vp.read(|s| s.prefs.show);
        if show.safe_rect {
            draw_safe_rect(&painter, rect, &p);
        }
        if let Gesture::Marquee { start, now } | Gesture::ToolBox { start, now } = &self.gesture {
            let r = Rect::from_two_pos(*start, *now);
            painter.rect_filled(r, CornerRadius::ZERO, p.accent_bright.gamma_multiply(0.12));
            painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, p.accent_bright), StrokeKind::Inside);
        }
        self.draw_brush(&painter, rect, size, ppp, &p, &ctx);
        self.draw_gizmo(&painter, rect, size, ppp, &p);
        self.drop_target(ui, rect, size, ppp);
        if let Some((text_, at)) = self.vp.read(|s| s.status.clone())
            && at.elapsed().as_secs_f32() < 2.0
        {
            // on a pill like the stats: the scene behind it may be any colour, the theme light or dark
            let font = Font::Small.id();
            let shown = Tx::ui(&text_).text(&ctx);
            let w = text::width(&ctx, &font, Tx::raw(&shown)).min(rect.width() - 32.0);
            let bg = Rect::from_min_max(
                pos2(rect.min.x + 8.0, rect.max.y - 8.0 - Font::Small.line() - 6.0),
                pos2(rect.min.x + 8.0 + w + 16.0, rect.max.y - 8.0),
            );
            painter.rect_filled(bg, CornerRadius::same(6), p.overlay.gamma_multiply(0.92));
            text::paint(
                &painter,
                &font,
                pos2(bg.min.x + 8.0, bg.center().y),
                Align2::LEFT_CENTER,
                p.text_secondary,
                Tx::raw(&shown),
                Some(w),
                Clip::End,
            );
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        if show.stats {
            local_stats_pill(&painter, rect, &self.vp, finished.as_ref(), &p);
        }
        self.local_axis_gizmo(ui, rect, &p);
        self.pills(ui, rect, cx);
        if renderer.as_ref().is_some_and(|r| r.finished().stats.placeholders > 0) {
            // meshes still loading: look again soon
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// A tick while the Level tab is not drawn: the camera is kept, no scene pass runs.
    pub fn inactive(&mut self, cx: &mut DocCx<'_>) {
        self.vp.sync(cx.editor);
        self.gesture = Gesture::None;
        if self.vp.read(|s| s.ghost.is_some()) {
            self.vp.with(|s| s.ghost = None);
        }
    }

    /// The keys the document takes after the actions: while the camera flies, W, A, S, D, Q and
    /// E are the camera's.
    pub fn shortcut(&mut self, _cx: &mut DocCx<'_>, chord: &Chord) -> bool {
        self.vp.read(|s| s.nav.flying()) && fly_key(chord.key).is_some()
    }

    /// What `state` / `ui.list` answer about it.
    #[must_use]
    pub fn mcp_state(&self) -> Value {
        let mut out = crate::cmd::view_state(&self.vp);
        if let Value::Object(m) = &mut out {
            let r = self.rect;
            m.insert("rect".into(), json!([r.min.x, r.min.y, r.width(), r.height()]));
            m.insert("frame".into(), json!(self.vp.renderer().map_or(0, |r| r.finished().frame)));
            m.insert("serial".into(), json!(self.serial));
        }
        out
    }

    fn texture_of(&mut self, gpu: &egui_wgpu::RenderState, t: &gz_render::TargetHandle) -> egui::TextureId {
        let mut r = gpu.renderer.write();
        match self.texture {
            Some((id, serial)) if serial == t.serial => id,
            Some((id, _)) => {
                r.update_egui_texture_from_wgpu_texture(&gpu.device, &t.view, wgpu::FilterMode::Linear, id);
                self.texture = Some((id, t.serial));
                id
            }
            None => {
                let id = r.register_native_texture(&gpu.device, &t.view, wgpu::FilterMode::Linear);
                self.texture = Some((id, t.serial));
                id
            }
        }
    }

    // ---- the mouse ---------------------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn input(&mut self, ctx: &egui::Context, canvas: &egui::Response, rect: Rect, ppp: f32, size: [u32; 2]) {
        // a gesture fed over MCP (`viewport.gesture`) stands for the mouse while it lasts
        let fed = self.vp.gesture_step();
        let real = fed.is_none();
        let mut motion = None;
        let mut carried = None;
        let (b, pressed, released, delta, pos, wheel, dt, over) = match fed {
            Some(f) => {
                let at = rect.min + f.step.at.to_vec2();
                let delta = self.fed_at.map_or(Vec2::ZERO, |p| at - p);
                self.fed_at = (!f.last).then_some(at);
                if !f.last {
                    ctx.request_repaint();
                }
                let b = Buttons {
                    left: f.step.left,
                    right: f.step.right,
                    middle: f.step.middle,
                    shift: f.mods.shift,
                    ctrl: f.mods.ctrl,
                    alt: f.mods.alt,
                };
                carried = f.payload.map(|p| (p, f.step.released));
                (b, f.step.pressed, f.step.released, delta, Some(at), 0.0, ctx.input(|i| i.stable_dt), true)
            }
            None => {
                motion = ctx.input(|i| i.pointer.motion());
                let (b, pressed, released, delta, pos, wheel, dt) = ctx.input(|i| {
                    let m = i.modifiers;
                    let b = Buttons {
                        left: i.pointer.primary_down(),
                        right: i.pointer.secondary_down(),
                        middle: i.pointer.middle_down(),
                        shift: m.shift,
                        ctrl: m.ctrl || m.command,
                        alt: m.alt,
                    };
                    (
                        b,
                        i.pointer.any_pressed(),
                        i.pointer.any_released(),
                        i.pointer.delta(),
                        i.pointer.latest_pos(),
                        wheel_steps(i),
                        i.stable_dt,
                    )
                });
                (b, pressed, released, delta, pos, wheel, dt, canvas.hovered() || canvas.dragged())
            }
        };
        let latest = pos;
        let pos = pos.unwrap_or(rect.center());
        // a pointer outside is pinned to the canvas's edge
        let clamped = pos2(pos.x.clamp(rect.min.x, rect.max.x), pos.y.clamp(rect.min.y, rect.max.y));
        let px = G2::new((clamped.x - rect.min.x) * ppp, (clamped.y - rect.min.y) * ppp);
        let (dx, dy) = (delta.x * ppp, delta.y * ppp);
        let ray = self.vp.read(|s| s.nav.ray(size, px));
        self.pointer = if over { ray.map(|r| (clamped, r)) } else { None };
        let local = clamped - rect.min.to_vec2();

        // a drop fed over MCP: the ghost while it is carried, the placing when it is let go
        if let Some((payload, let_go)) = carried {
            if let Some(ray) = ray {
                self.carry(&payload, clamped, ray, ppp);
                if let_go {
                    self.drop_payload(&payload, ray);
                }
            }
            self.last_frame(ctx, latest, dt);
            return;
        }

        // the gizmo handle under the mouse
        if matches!(self.gesture, Gesture::None) {
            self.hover = if over { self.hover_status(local, size, ppp) } else { Status::None };
        }

        if wheel != 0.0 && (over || !matches!(self.gesture, Gesture::None)) {
            self.vp.with(|s| {
                if let Some(status) = s.nav.wheel(wheel) {
                    s.status = Some((status, Instant::now()));
                }
            });
        }

        match &mut self.gesture {
            Gesture::None => {
                // a handle of the gizmo takes a left press before the camera (an Alt drag of it copies)
                let handle = b.left && !b.right && !b.middle && self.hover != Status::None;
                if pressed && over && b.any_button() {
                    if handle && let Some(ray) = ray {
                        self.mode_press(&b, ray, clamped, local, size, ppp);
                    } else if self.vp.with(|s| s.nav.move_start(b)) {
                        self.gesture = Gesture::Camera;
                        if real {
                            self.hold = Some(pos);
                        }
                    } else if b.left
                        && let Some(ray) = ray
                    {
                        self.mode_press(&b, ray, clamped, local, size, ppp);
                    }
                }
            }
            Gesture::Camera => {
                // the cursor held in place: the camera turns by the mouse's own motion
                let (dx, dy) = match self.hold {
                    Some(hold) if real => {
                        let d = held_delta(motion, latest, hold, ppp);
                        (d.x, d.y)
                    }
                    _ => (dx, dy),
                };
                if dx != 0.0 || dy != 0.0 {
                    self.vp.with(|s| {
                        let aspect = s.aspect();
                        s.nav.process(b, dx, dy, aspect)
                    });
                }
                if (released && self.vp.with(|s| s.nav.move_end(b)))
                    || (!b.any_button() && !self.vp.read(|s| s.nav.is_moving()))
                {
                    self.gesture = Gesture::None;
                }
            }
            Gesture::Press { start, click } => {
                let start = *start;
                match press_step(&b, start, clamped, click.is_some()) {
                    PressStep::Wait => {}
                    PressStep::Click => {
                        let click = click.take();
                        self.gesture = Gesture::None;
                        if let Some(c) = click {
                            self.run_click(c);
                        }
                    }
                    PressStep::Nothing => self.gesture = Gesture::None,
                    PressStep::Marquee => self.gesture = Gesture::Marquee { start, now: clamped },
                    PressStep::Drag => *click = None,
                    PressStep::Camera => {
                        *click = None;
                        // a left drag no mode took is the camera's, and selects nothing
                        if self.vp.with(|s| s.nav.left_drag_start(b)) {
                            self.gesture = Gesture::Camera;
                            if real {
                                self.hold = Some(start);
                            }
                        }
                    }
                }
            }
            Gesture::Marquee { start, now } => {
                *now = clamped;
                if !b.left {
                    let (a, z) = (*start, *now);
                    self.gesture = Gesture::None;
                    let to_px = |q: Pos2| G2::new((q.x - rect.min.x) * ppp, (q.y - rect.min.y) * ppp);
                    match scene_mode(&self.editor).and_then(SceneMode::layer) {
                        Some(layer) => {
                            crate::bulktools::marquee(
                                &self.editor,
                                &self.vp,
                                size,
                                to_px(a),
                                to_px(z),
                                layer,
                            );
                        }
                        None => marquee_select(&self.editor, &self.vp, size, to_px(a), to_px(z)),
                    }
                }
            }
            Gesture::ToolBox { start, now } => {
                *now = clamped;
                if !b.left {
                    let (a, z) = (*start, *now);
                    self.gesture = Gesture::None;
                    let to_px = |q: Pos2| G2::new((q.x - rect.min.x) * ppp, (q.y - rect.min.y) * ppp);
                    crate::objtools::box_pick_end(&self.editor, &self.vp, size, to_px(a), to_px(z));
                }
            }
            Gesture::AiPaint(paint) => {
                if !b.left {
                    crate::bulktools::paint_end(&self.editor);
                    self.gesture = Gesture::None;
                } else if (dx != 0.0 || dy != 0.0)
                    && let Some(ray) = ray
                {
                    crate::bulktools::ai_step(&self.editor, &self.vp, paint, ray, false);
                }
            }
            Gesture::Foliage(stroke) => {
                if !b.left {
                    let Gesture::Foliage(stroke) = std::mem::replace(&mut self.gesture, Gesture::None) else {
                        return;
                    };
                    crate::bulktools::foliage_end(
                        &self.editor,
                        &self.vp,
                        stroke.erase,
                        stroke.spots,
                        stroke.picking,
                    );
                } else if (clamped - stroke.last).length() > 6.0
                    && let Some(ray) = ray
                {
                    stroke.last = clamped;
                    crate::bulktools::foliage_spot(&self.editor, &stroke.spots, &stroke.picking, ray);
                }
            }
            Gesture::ToolPaint(paint) => {
                if !b.left {
                    // the whole paint is one undo step
                    if let Ok(svc) = SceneService::of(&self.editor) {
                        svc.store.seal();
                    }
                    self.gesture = Gesture::None;
                } else if (dx != 0.0 || dy != 0.0)
                    && let Some(ray) = ray
                {
                    crate::objtools::paint_step(&self.editor, &self.vp, paint, ray);
                }
            }
            Gesture::Transform(drag) => {
                if !b.left {
                    // one drag is one undo step
                    drag.svc.store.seal();
                    self.gesture = Gesture::None;
                } else if (dx != 0.0 || dy != 0.0)
                    && let Some(ray) = ray
                {
                    apply_drag(&self.vp, drag, &ray, local, &self.editor);
                }
            }
        }
        self.last_frame(ctx, latest, dt);
    }

    /// What every frame of input ends with: the held cursor, the flying keys, the next frame
    /// while a gesture lasts.
    fn last_frame(&mut self, ctx: &egui::Context, latest: Option<Pos2>, dt: f32) {
        self.hold_cursor(ctx, latest);
        // flying: WASD and QE held move the camera every frame
        if self.vp.read(|s| s.nav.flying()) {
            let down: Vec<FlyKey> = ctx.input(|i| {
                [
                    (Key::W, FlyKey::W),
                    (Key::S, FlyKey::S),
                    (Key::A, FlyKey::A),
                    (Key::D, FlyKey::D),
                    (Key::E, FlyKey::E),
                    (Key::Q, FlyKey::Q),
                ]
                .into_iter()
                .filter(|(k, _)| i.key_down(*k))
                .map(|(_, f)| f)
                .collect()
            });
            let moved = self.vp.with(|s| {
                s.nav.set_keys(&down);
                s.nav.update(dt.min(0.1))
            });
            if moved || !down.is_empty() {
                ctx.request_repaint();
            }
        }
        if !matches!(self.gesture, Gesture::None) {
            ctx.request_repaint();
        }
    }

    /// The gizmo's axes as a drag of `kind` uses them.
    fn axes(&self, kind: GizmoType, anchor: &Anchor) -> [Vec3; 3] {
        gizmo_axes(kind, anchor, self.vp.read(|s| s.prefs.space))
    }

    /// The gizmo's handles on the screen now, in points from the pane's corner; `None` while no
    /// gizmo is up.
    fn shapes(&self, size: [u32; 2], ppp: f32) -> Option<(GizmoType, Anchor, Shapes)> {
        let mode = scene_mode(&self.editor)?;
        if !matches!(mode, SceneMode::Select | SceneMode::AiMap) {
            return None;
        }
        let (kind, anchor, cam) = self.vp.read(|s| (s.gizmo, s.anchor, s.nav.camera()));
        if kind == GizmoType::None {
            return None;
        }
        let anchor = anchor?;
        let at = gizmo::screen_position(&cam, size, anchor.position)?;
        let vp_m = cam.view_proj(size);
        let proj = |w: Vec3| gz_render::project(&vp_m, size, w).map(|q| G2::new(q.x / ppp, q.y / ppp));
        let axes = self.axes(kind, &anchor);
        let shapes = gizmo::shapes(kind, at, axes, cam.forward, &proj, mode == SceneMode::AiMap)?;
        Some((kind, anchor, shapes))
    }

    /// The gizmo handle under the pointer.
    fn hover_status(&self, local: Pos2, size: [u32; 2], ppp: f32) -> Status {
        match self.shapes(size, ppp) {
            Some((kind, _, s)) => gizmo::hit(kind, &s, G2::new(local.x, local.y)),
            None => Status::None,
        }
    }

    /// A left press the camera did not take: what the mode does with it.
    fn mode_press(&mut self, b: &Buttons, ray: Ray, at: Pos2, local: Pos2, size: [u32; 2], ppp: f32) {
        self.serial += 1;
        let mode = scene_mode(&self.editor).unwrap_or_default();
        // a gizmo handle under the pointer: a drag of it, in Select and in the AI map's mode
        if self.hover != Status::None
            && let Some(drag) = self.start_drag(b, ray, local, size, ppp)
        {
            self.gesture = Gesture::Transform(Box::new(drag));
            return;
        }
        let press = |click| Gesture::Press { start: at, click };
        self.gesture = match mode {
            SceneMode::Select => press(Some(Click::Select { ray, b: *b })),
            SceneMode::Place => {
                match crate::objtools::place_press(&self.editor, &self.vp, ray, *b, self.serial) {
                    crate::objtools::ToolGesture::None => press(Some(Click::Place { ray })),
                    _ => press(None),
                }
            }
            SceneMode::Zones => {
                match crate::objtools::place_press(&self.editor, &self.vp, ray, *b, self.serial) {
                    crate::objtools::ToolGesture::Paint(p) => Gesture::ToolPaint(p),
                    crate::objtools::ToolGesture::BoxPick => Gesture::ToolBox { start: at, now: at },
                    crate::objtools::ToolGesture::Done => press(None),
                    crate::objtools::ToolGesture::None => press(Some(Click::Select { ray, b: *b })),
                }
            }
            SceneMode::AiMap | SceneMode::Details | SceneMode::Wallmarks if b.ctrl => {
                let layer = mode.layer().unwrap_or(ObjClass::AiMap);
                press(Some(Click::Layer { ray, b: *b, layer }))
            }
            SceneMode::AiMap => {
                Gesture::AiPaint(crate::bulktools::ai_press(&self.editor, &self.vp, ray, self.serial))
            }
            SceneMode::Details => {
                let stroke = BrushStroke {
                    erase: b.shift,
                    spots: Arc::new(Mutex::new(Vec::new())),
                    picking: Arc::new(AtomicUsize::new(0)),
                    last: at,
                };
                crate::bulktools::foliage_spot(&self.editor, &stroke.spots, &stroke.picking, ray);
                Gesture::Foliage(Box::new(stroke))
            }
            SceneMode::Wallmarks => press(Some(Click::Decal { ray, b: *b })),
        };
    }

    /// A left click, the button up: what its press waited to do.
    fn run_click(&mut self, click: Click) {
        match click {
            Click::Select { ray, b } => click_select(&self.editor, &self.vp, ray, b),
            Click::Layer { ray, b, layer } => {
                crate::bulktools::click(&self.editor, &self.vp, ray, b, layer);
            }
            Click::Place { ray } => {
                let Some(what) = crate::place::armed(&self.editor) else { return };
                let ed = self.editor.clone();
                crate::gesture::spawn(&self.editor, async move {
                    crate::place::place_at(&ed, &what, ray).await;
                });
            }
            Click::Decal { ray, b } => crate::bulktools::mark_place(&self.editor, &self.vp, ray, b.shift),
        }
    }

    /// While the camera has the mouse the cursor is hidden and held where the press was, so the
    /// camera turns without end; it is let go there when the camera lets the mouse go.
    fn hold_cursor(&mut self, ctx: &egui::Context, at: Option<Pos2>) {
        let Some(hold) = self.hold else { return };
        let ended = !matches!(self.gesture, Gesture::Camera);
        // another window in front: the cursor is the system's again at once, where it is
        let left = !ctx.input(|i| i.focused);
        if ended || left {
            self.hold = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::None));
            if !left {
                ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(hold));
            }
            return;
        }
        ctx.set_cursor_icon(egui::CursorIcon::None);
        ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(egui::CursorGrab::Confined));
        if at.is_none_or(|p| (p - hold).length() > 0.25) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorPosition(hold));
        }
    }

    fn start_drag(&mut self, b: &Buttons, ray0: Ray, press: Pos2, size: [u32; 2], ppp: f32) -> Option<Drag> {
        let (kind, anchor, shapes) = self.shapes(size, ppp)?;
        let svc = SceneService::of(&self.editor).ok()?;
        let snap = svc.store.snapshot();
        let sup = gizmo::supports_of(&snap);
        let allowed = match kind {
            GizmoType::Move => sup[0],
            GizmoType::Rotate => sup[1],
            GizmoType::Scale => sup[2],
            GizmoType::None => false,
        };
        if !allowed {
            return None;
        }
        let (space, cam) = self.vp.read(|s| (s.prefs.space, s.nav.camera()));
        let axes = self.axes(kind, &anchor);
        let status = self.hover;
        let axis = match status {
            Status::Screen => -cam.forward,
            s => s.axis().map_or(Vec3::Y, |i| axes[i]),
        };
        let edge = kind == GizmoType::Rotate && ray0.dir.dot(axis).abs() < 0.12;
        let merge = SmolStr::new(format!("viewport.gizmo.{}", self.serial));
        let base = Drag {
            kind,
            status,
            ray0,
            press,
            pivot: anchor.position,
            axes,
            space,
            shapes,
            edge,
            held: Vec::new(),
            points: Vec::new(),
            nodes: Vec::new(),
            angle: 0.0,
            applied: 0.0,
            merge,
            svc: svc.clone(),
            copies: None,
        };
        if snap.tools.mode == SceneMode::AiMap {
            let nodes = crate::bulktools::held_nodes(&self.editor);
            return (!nodes.is_empty()).then_some(Drag { nodes, ..base });
        }
        let ids: Vec<ObjId> = snap
            .selected_in_order()
            .into_iter()
            .filter(|id| snap.get(*id).is_some_and(|o| o.flags.visible && o.editable() && snap.changeable(o)))
            .collect();
        let held = held_of(&svc, &snap, &ids);
        if held.is_empty() {
            return None;
        }
        // Alt: the drag moves copies, made first as an undo step of their own
        let copies = (b.alt && kind != GizmoType::None).then(|| {
            let slot: Arc<Mutex<Option<Vec<ObjId>>>> = Arc::new(Mutex::new(None));
            let (ed, svc2, ids2, out) = (self.editor.clone(), svc.clone(), ids.clone(), slot.clone());
            crate::gesture::spawn(&self.editor, async move {
                let made = gz_scene::edit::duplicate(&svc2, ids2, Vec3::ZERO, ed.ui_origin()).await;
                let copies = match made {
                    Ok(_) => svc2.store.snapshot().selected_in_order(),
                    Err(e) => {
                        ed.log.error(e.message);
                        Vec::new()
                    }
                };
                *out.lock() = Some(copies);
                ed.exec.wake().ping();
            });
            slot
        });
        let way_ids: Vec<ObjId> = held.iter().map(|h| h.id).collect();
        let points = crate::objtools::held_points(&self.editor, &way_ids)
            .into_iter()
            .filter(|(_, pts)| !pts.is_empty())
            .collect();
        Some(Drag { held, points, copies, ..base })
    }

    // ---- the drop target ---------------------------------------------------------------------

    /// A drag of something placeable over the viewport: its ghost on the surface under the
    /// pointer, and the placing when it is let go.
    fn drop_target(&mut self, ui: &mut Ui, rect: Rect, size: [u32; 2], ppp: f32) {
        let ctx = ui.ctx().clone();
        let carried = gz_ui::drop::dragged(&ctx).filter(|p| !Placeable::of_payload(p).is_empty());
        let dropped = gz_ui::drop::Zone::new("Viewport", rect, &crate::place::takes).take(ui);
        let over = ctx.pointer_latest_pos().filter(|p| rect.contains(*p));
        let ray_at = |at: Pos2| {
            let px = G2::new((at.x - rect.min.x) * ppp, (at.y - rect.min.y) * ppp);
            self.vp.read(|s| s.nav.ray(size, px))
        };
        if let Some(payload) = dropped {
            if let Some(ray) = over.and_then(ray_at) {
                self.drop_payload(&payload, ray);
            }
            return;
        }
        match (carried, over) {
            (Some(payload), Some(at)) => {
                if let Some(ray) = ray_at(at) {
                    self.carry(&payload, at, ray, ppp);
                    let title =
                        Placeable::of_payload(&payload).first().map(Placeable::title_tx).unwrap_or_default();
                    let p = palette(&ctx);
                    text::paint(
                        &ui.painter_at(rect),
                        &Font::Small.id(),
                        at + vec2(14.0, 14.0),
                        Align2::LEFT_TOP,
                        p.text,
                        tr!("Place {title}", title = title),
                        None,
                        Clip::End,
                    );
                }
            }
            _ => {
                if self.vp.read(|s| s.ghost.is_some()) && !self.fed_drop() {
                    self.vp.with(|s| s.ghost = None);
                    self.ground.forget();
                }
            }
        }
    }

    fn fed_drop(&self) -> bool {
        self.vp.gesture.lock().is_some()
    }

    /// The ghost of what a drag carries, where it would land now.
    fn carry(&mut self, payload: &gz_ui::drop::Payload, at: Pos2, ray: Ray, _ppp: f32) {
        self.ground.track(&self.editor, at, ray);
        let align = self.vp.read(|s| s.prefs.snaps.align);
        if let Some(ghost) = self.ground.at().and_then(|hit| crate::place::ghost_of(payload, hit, align))
            && self.vp.read(|s| s.ghost.as_ref() != Some(&ghost))
        {
            self.vp.with(|s| s.ghost = Some(ghost));
        }
    }

    /// What a drag carried, placed where it was let go, each thing one undo step, selected.
    fn drop_payload(&mut self, payload: &gz_ui::drop::Payload, ray: Ray) {
        self.vp.with(|s| s.ghost = None);
        self.ground.forget();
        let list = Placeable::of_payload(payload);
        let ed = self.editor.clone();
        crate::gesture::spawn(&self.editor, async move {
            for what in &list {
                crate::place::place_at(&ed, what, ray).await;
            }
        });
    }

    // ---- what is drawn over the picture --------------------------------------------------------

    /// The Paint modes' brush: a ring on the ground under the pointer, the danger colour while
    /// it erases.
    fn draw_brush(
        &mut self,
        painter: &egui::Painter,
        rect: Rect,
        size: [u32; 2],
        ppp: f32,
        p: &Palette,
        ctx: &egui::Context,
    ) {
        let Some(mode) = scene_mode(&self.editor) else { return };
        let radius = match mode {
            SceneMode::Details => self.vp.read(|s| s.brush.radius),
            SceneMode::AiMap => crate::bulktools::ai_brush_radius(&self.editor),
            _ => return,
        };
        let Some((_, ray)) = self.pointer else {
            self.ground.forget();
            return;
        };
        let at = self.pointer.map_or(rect.center(), |(a, _)| a);
        self.ground.track(&self.editor, at, ray);
        let Some((centre, up)) = self.ground.at() else { return };
        let cam = self.vp.read(|s| s.nav.camera());
        let vp_m = cam.view_proj(size);
        let u = up.any_orthonormal_vector();
        let w = up.cross(u);
        let ring: Vec<Pos2> = (0..=64)
            .filter_map(|k| {
                let t = k as f32 / 64.0 * std::f32::consts::TAU;
                gz_render::project(&vp_m, size, centre + (u * t.cos() + w * t.sin()) * radius)
                    .map(|q| pos2(rect.min.x + q.x / ppp, rect.min.y + q.y / ppp))
            })
            .collect();
        let erase = mode == SceneMode::Details
            && (ctx.input(|i| i.modifiers.shift) || matches!(&self.gesture, Gesture::Foliage(s) if s.erase));
        let c = if erase { p.error } else { p.accent_bright };
        if ring.len() > 2 {
            painter.add(egui::Shape::line(ring, Stroke::new(2.0, c)));
        }
        ctx.request_repaint_after(Duration::from_millis(50));
    }

    #[allow(clippy::too_many_lines)]
    fn draw_gizmo(&self, painter: &egui::Painter, rect: Rect, size: [u32; 2], ppp: f32, p: &Palette) {
        let Some((kind, anchor, s)) = self.shapes(size, ppp) else { return };
        let off = rect.min.to_vec2();
        let at = |v: G2| pos2(v.x, v.y) + off;
        let (status, drag) = match &self.gesture {
            Gesture::Transform(d) => (d.status, Some(d)),
            _ => (self.hover, None),
        };
        let colour = |st: Status| match st {
            Status::X | Status::YZ => p.axis_x,
            Status::Y | Status::XZ => p.axis_y,
            Status::Z | Status::XY => p.axis_z,
            _ => p.text_secondary,
        };
        let paint_of = |st: Status| if status == st { p.text } else { colour(st) };
        let width = |st: Status| if status == st { 4.0 } else { 3.0 };
        // the planes first, under the arrows
        for (st, q) in &s.planes {
            let pts: Vec<Pos2> = q.iter().map(|v| at(*v)).collect();
            let fill = paint_of(*st).gamma_multiply(if status == *st { 0.55 } else { 0.3 });
            painter.add(egui::Shape::convex_polygon(pts.clone(), fill, Stroke::new(1.0, paint_of(*st))));
        }
        for (st, a, b, head) in &s.axes {
            let c = paint_of(*st);
            match kind {
                GizmoType::Scale => {
                    painter.line_segment([at(*a), at(*b)], Stroke::new(width(*st), c));
                    painter.rect_filled(
                        Rect::from_center_size(at(*b), Vec2::splat(9.0)),
                        CornerRadius::same(1),
                        c,
                    );
                }
                _ => {
                    painter.line_segment([at(*a), at(*head)], Stroke::new(width(*st), c));
                    let (h, tip) = (at(*head), at(*b));
                    let dir = (tip - h).normalized();
                    let side = vec2(-dir.y, dir.x) * ((tip - h).length() * 0.42).clamp(4.0, 8.0);
                    painter.add(egui::Shape::convex_polygon(vec![tip, h + side, h - side], c, Stroke::NONE));
                }
            }
        }
        for (st, ring) in &s.rings {
            let pts: Vec<Pos2> = ring.iter().map(|v| at(*v)).collect();
            painter.add(egui::Shape::line(pts, Stroke::new(width(*st), paint_of(*st))));
        }
        if let Some(r) = s.screen_ring {
            let c = if status == Status::Screen { p.text } else { p.text_secondary };
            painter.circle_stroke(
                at(s.centre),
                r,
                Stroke::new(if status == Status::Screen { 3.0 } else { 2.0 }, c),
            );
        }
        if s.centre_handle {
            let hot = matches!(status, Status::Screen | Status::Xyz);
            let c = if hot { p.text } else { p.text_secondary };
            painter.rect_filled(
                Rect::from_center_size(at(s.centre), Vec2::splat(gizmo::CENTRE * 2.0 - 2.0)),
                CornerRadius::same(2),
                c.gamma_multiply(if hot { 0.9 } else { 0.6 }),
            );
        } else {
            painter.circle_filled(at(s.centre), 2.5, p.text_secondary);
        }
        // a turn under way: its arc on the ring, and the angle
        if let Some(d) = drag.filter(|d| d.kind == GizmoType::Rotate) {
            let cam = self.vp.read(|st| st.nav.camera());
            // the scene's angles turn the other way round an axis: the readout says what Details will
            let deg = -d.angle.to_degrees();
            if !d.edge
                && let Some(gat) = gizmo::screen_position(&cam, size, anchor.position)
            {
                let (axis, radius) = match d.status {
                    Status::Screen => (-cam.forward, gizmo::SCREEN_RING),
                    st => (st.axis().map_or(Vec3::Y, |i| d.axes[i]), gizmo::RING),
                };
                let u = axis.any_orthonormal_vector();
                let w = axis.cross(u);
                let start = gizmo::ray_plane(&d.ray0, d.pivot, axis).map(|h| h - d.pivot);
                if let Some(v0) = start {
                    let a0 = v0.dot(w).atan2(v0.dot(u));
                    let vp_m = cam.view_proj(size);
                    let proj = |v: Vec3| {
                        gz_render::project(&vp_m, size, v)
                            .map(|q| pos2(rect.min.x + q.x / ppp, rect.min.y + q.y / ppp))
                    };
                    let n = ((d.angle.abs() / 0.05).ceil() as usize).clamp(2, 128);
                    let mut fan: Vec<Pos2> = Vec::with_capacity(n + 2);
                    if let Some(c) = proj(gat) {
                        fan.push(c);
                    }
                    for k in 0..=n {
                        let t = a0 + d.angle * k as f32 / n as f32;
                        if let Some(q) = proj(gat + (u * t.cos() + w * t.sin()) * radius) {
                            fan.push(q);
                        }
                    }
                    let c = colour(d.status);
                    if fan.len() > 2 {
                        let edge: Vec<Pos2> = fan[1..].to_vec();
                        painter.add(egui::Shape::Mesh(Arc::new(fan_mesh(&fan, c.gamma_multiply(0.25)))));
                        painter.add(egui::Shape::line(edge, Stroke::new(2.0, c)));
                    }
                }
            }
            let label = format!("{deg:.1}\u{b0}");
            let place = at(s.centre) + vec2(16.0, -24.0);
            let galley = painter.layout_no_wrap(label, Font::Body.id(), p.text);
            let bg = Rect::from_min_size(place, galley.size()).expand2(vec2(6.0, 3.0));
            painter.rect_filled(bg, CornerRadius::same(4), p.overlay.gamma_multiply(0.92));
            painter.galley(place, galley, p.text);
        }
    }

    /// The 64 pt clickable axis gizmo at the top right; hidden under 480 pt and when the view is
    /// too short for it.
    fn local_axis_gizmo(&mut self, ui: &mut Ui, rect: Rect, p: &Palette) {
        if rect.width() < 480.0 {
            return;
        }
        let size = 64.0;
        let min = pos2(rect.max.x - size - 12.0, rect.min.y + 44.0);
        if min.y + size > rect.max.y - 8.0 {
            return;
        }
        let r = Rect::from_min_size(min, Vec2::splat(size));
        let resp = ui.interact(r, ui.make_persistent_id("gz-ui-viewport.axis_gizmo"), Sense::click());
        let centre = r.center();
        let reach = size * 0.5 - 9.0;
        let (right, up, forward) = self.vp.read(|s| (s.nav.right(), s.nav.up(), s.nav.direction()));
        struct End {
            target: Target,
            pos: Pos2,
            depth: f32,
            color: Color32,
            label: Option<&'static str>,
        }
        let mut ends = Vec::with_capacity(6);
        // a view looks from the end it is named by: Right from +X, Top from +Y, Back from +Z
        for (dir, pos_t, neg_t, color, label) in [
            (Vec3::X, Target::Right, Target::Left, p.axis_x, "X"),
            (Vec3::Y, Target::Top, Target::Bottom, p.axis_y, "Y"),
            (Vec3::Z, Target::Back, Target::Front, p.axis_z, "Z"),
        ] {
            let screen = vec2(dir.dot(right) * reach, -dir.dot(up) * reach);
            let depth = dir.dot(forward);
            ends.push(End { target: pos_t, pos: centre + screen, depth, color, label: Some(label) });
            ends.push(End { target: neg_t, pos: centre - screen, depth: -depth, color, label: None });
        }
        // the far ends first
        ends.sort_by(|a, b| b.depth.total_cmp(&a.depth));
        let mouse = ui.ctx().input(|i| i.pointer.hover_pos());
        let mut hot = None;
        if resp.hovered()
            && let Some(m) = mouse
        {
            for (i, e) in ends.iter().enumerate() {
                if (m - e.pos).length() <= 8.0 {
                    hot = Some(i);
                }
            }
        }
        let painter = ui.painter_at(rect);
        if resp.hovered() || resp.is_pointer_button_down_on() {
            painter.circle_filled(centre, size * 0.5, p.overlay.gamma_multiply(0.75));
        }
        for (i, e) in ends.iter().enumerate() {
            match e.label {
                Some(l) => {
                    painter.line_segment([centre, e.pos], Stroke::new(2.0, e.color));
                    painter.circle_filled(e.pos, 7.0, e.color);
                    painter.text(e.pos, Align2::CENTER_CENTER, l, Font::Small.id(), p.canvas);
                }
                None => {
                    painter.circle_filled(e.pos, 5.0, e.color.gamma_multiply(0.45));
                }
            }
            if hot == Some(i) {
                painter.circle_stroke(
                    e.pos,
                    if e.label.is_some() { 8.5 } else { 6.5 },
                    Stroke::new(1.5, p.text),
                );
            }
        }
        painter.circle_filled(centre, 2.5, p.text_secondary);
        // a click on an end looks from it; anywhere else on the gizmo goes back to the free view
        let target = hot.and_then(|i| ends.get(i)).map_or(Target::Perspective, |e| e.target);
        let tip = match target {
            Target::Perspective => "Perspective: back to the free view",
            Target::Front => "Front view",
            Target::Back => "Back view",
            Target::Left => "Left view",
            Target::Right => "Right view",
            Target::Top => "Top view",
            Target::Bottom => "Bottom view",
        };
        gz_ui::audit::control(ui.ctx(), "Axis gizmo", true, None, Some(tip), r);
        if resp.tip(tip).clicked() {
            self.vp.with(|s| s.nav.set_target(target));
        }
    }

    /// The pills: the mode control, the camera, the view mode and what is shown on the left; the
    /// transform tool, its space and the snapping on the right. One overlay toolbar, so a narrow
    /// viewport folds the right side into its overflow first.
    #[allow(clippy::too_many_lines)]
    fn pills(&mut self, ui: &mut Ui, rect: Rect, cx: &mut DocCx<'_>) {
        let ed = self.editor.clone();
        let vp = self.vp.clone();
        let sup = ed
            .project_opt()
            .and_then(|s| s.services.get::<SceneService>())
            .map_or([true; 3], |svc| gizmo::supports_of(&svc.store.snapshot()));
        // a transform the selection cannot take falls back to plain selection
        let reset = vp.read(|s| match s.gizmo {
            GizmoType::Move => !sup[0],
            GizmoType::Rotate => !sup[1],
            GizmoType::Scale => !sup[2],
            GizmoType::None => false,
        });
        if reset {
            vp.with(|s| s.gizmo = GizmoType::None);
        }
        let mode = scene_mode(&ed);
        // Paint (3) goes back to the layer painted last
        if let Some(m) = mode.filter(|m| m.layer().is_some())
            && vp.read(|s| s.paint_layer != m)
        {
            vp.with(|s| s.paint_layer = m);
        }
        let (view_target, ortho, view_mode, space) =
            vp.read(|s| (s.nav.current(), s.nav.ortho, s.prefs.mode, s.prefs.space));
        let bar = Rect::from_min_max(
            pos2(rect.min.x + 8.0, rect.min.y + 4.0),
            pos2(
                (rect.max.x - 8.0).max(rect.min.x + 32.0),
                rect.min.y + 4.0 + ToolbarStyle::Overlay.height(),
            ),
        );
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-viewport.pills")
                .max_rect(bar)
                .layout(Layout::left_to_right(Align::Center)),
        );
        {
            let mut tb = Toolbar::begin(&mut child, "viewport", ToolbarStyle::Overlay);
            tb.group(4);
            let mut current = mode_index(mode);
            let tip = "Select | Place | Paint | Zones (1, 2, 3, 4): what a click in the viewport does";
            if tb.segmented(
                "mode",
                "Mode",
                &mut current,
                &["Select", "Place", "Paint", "Zones"],
                tip,
                mode.is_some(),
            ) && let Some(id) = crate::actions::MODES.get(current)
            {
                cx.requests.action(id);
            }
            tb.group(3);
            let icon =
                if view_target == Target::Perspective && !ortho { Icon::Perspective } else { Icon::Ortho };
            tb.menu(
                "perspective",
                icon,
                // a view's name: "Back" is where the camera looks from
                gz_ui::Tx::ctx("Viewport", view_target.name()),
                "The camera's view, framing, bookmarks",
                Some("viewport:perspective"),
                true,
                |ui| menus::perspective_body(ui, cx),
            );
            tb.menu(
                "view_mode",
                view_mode.icon(),
                view_mode.title(),
                "How the scene is lit: Studio, Unlit, Game Lit, Wireframe",
                Some("viewport:view_mode"),
                true,
                |ui| menus::view_mode_body(ui, cx),
            );
            tb.menu(
                "show",
                Icon::Eye,
                "Show",
                "What the viewport draws over the scene",
                Some("viewport:show"),
                true,
                |ui| menus::show_body(ui, cx),
            );
            tb.spring();

            tb.group(2);
            for (id, icon) in [
                ("viewport.select_tool", Icon::Select),
                ("viewport.move_tool", Icon::Move),
                ("viewport.rotate_tool", Icon::Rotate),
                ("viewport.scale_tool", Icon::Scale),
            ] {
                pill_action(&mut tb, cx, id, icon, "");
            }
            let space_label = if space == Space::Local { "Local" } else { "World" };
            pill_action(
                &mut tb,
                cx,
                "viewport.toggle_space",
                if space == Space::Local { Icon::SceneObject } else { Icon::Globe },
                space_label,
            );

            tb.group(1);
            pill_action(&mut tb, cx, "viewport.surface_snap", Icon::SnapObject, "");
            tb.menu(
                "surface",
                Icon::None,
                "",
                "Surface Snap's options",
                Some("viewport:surface"),
                true,
                |ui| menus::surface_body(ui, cx),
            );
            for (kind, id, menu, request) in [
                (GizmoType::Move, "viewport.grid_snap", "grid", "viewport:grid"),
                (GizmoType::Rotate, "viewport.angle_snap", "angle", "viewport:angle"),
                (GizmoType::Scale, "viewport.scale_snap", "scale", "viewport:scale"),
            ] {
                tb.group(0);
                let step = vp.read(|s| s.step(kind));
                let icon = match kind {
                    GizmoType::Move => Icon::SnapGrid,
                    GizmoType::Rotate => Icon::SnapAngle,
                    _ => Icon::SnapScale,
                };
                pill_action(&mut tb, cx, id, icon, "");
                let value = menus::step_text(kind, step);
                let tip = match kind {
                    GizmoType::Move => "The grid step a move goes by",
                    GizmoType::Rotate => "The angle a turn goes by",
                    _ => "The step scaling goes by",
                };
                tb.menu(menu, Icon::None, value, tip, Some(request), true, |ui| {
                    menus::step_body(ui, cx, kind)
                });
            }
        }
    }
}

/// A pill of an action: its icon (and `label` when it has one), its title and keys as the tip,
/// off with the reason, checked as a toggle. Pressed, it runs.
fn pill_action(tb: &mut Toolbar<'_, '_>, cx: &mut DocCx<'_>, id: &str, icon: Icon, label: &'static str) {
    let Some(actions) = gz_app::Actions::of(cx.editor) else { return };
    let Some(a) = actions.table.read().get(id).copied() else { return };
    let keys = actions.table.read().keys_label(id);
    let (on, checked) =
        actions.with_cx(cx.editor, |acx| ((a.enabled)(acx), a.checked.is_some_and(|f| f(acx))));
    // the tip is put together from words already in the window's language
    let title = gz_ui::i18n::t(cx.ctx, a.title);
    let tip = match (on.on(), keys.is_empty()) {
        (false, _) => format!("{title}: {}", gz_ui::i18n::t(cx.ctx, on.why())),
        (true, true) => title,
        (true, false) => format!("{title} ({keys})"),
    };
    if tb.action(id, icon, label, Tx::raw(&tip), on.on(), checked, Kind::Ghost) {
        cx.requests.action(id);
    }
}

/// The mode control's segment of a scene mode: Select, Place, Paint (any painted layer), Zones.
fn mode_index(mode: Option<SceneMode>) -> usize {
    match mode {
        Some(SceneMode::Place) => 1,
        Some(m) if m.layer().is_some() => 2,
        Some(SceneMode::Zones) => 3,
        _ => 0,
    }
}

/// A fan of triangles from its first point.
fn fan_mesh(pts: &[Pos2], c: Color32) -> egui::Mesh {
    let mut m = egui::Mesh::default();
    for q in pts {
        m.colored_vertex(*q, c);
    }
    for i in 1..pts.len().saturating_sub(1) {
        m.add_triangle(0, i as u32, i as u32 + 1);
    }
    m
}

/// The scene's mode: what a press in the viewport does. `None` without a scene.
pub(crate) fn scene_mode(editor: &Editor) -> Option<SceneMode> {
    editor
        .project_opt()
        .and_then(|s| s.services.get::<SceneService>())
        .map(|svc| svc.store.snapshot().tools.mode)
}

/// The gizmo's axes for a tool: Scale works along the objects' own axes, the AI map's nodes
/// along the world's, the rest in the space the pill says.
pub(crate) fn gizmo_axes(kind: GizmoType, anchor: &Anchor, space: Space) -> [Vec3; 3] {
    if kind == GizmoType::Scale { anchor.axes(Space::Local) } else { anchor.axes(space) }
}

/// What a drag holds of `ids`, as they are now.
fn held_of(svc: &SceneService, snap: &gz_scene::SceneState, ids: &[ObjId]) -> Vec<Held> {
    ids.iter()
        .filter_map(|id| snap.get(*id))
        .map(|o| Held {
            id: o.id,
            position: o.position(),
            rotation: o.rotation(),
            scale: o.scale(),
            glow: o.class == ObjClass::Glow,
            dynamic: matches!(&o.data, ObjData::SceneObject(d) if svc.library.cached(&d.reference).is_some_and(|l| l.is_dynamic())),
        })
        .collect()
}

/// The wheel's turn this frame in notches, upward positive (egui's 40 points a line).
fn wheel_steps(i: &egui::InputState) -> f32 {
    i.events
        .iter()
        .map(|e| match e {
            egui::Event::MouseWheel { unit, delta, .. } => match unit {
                egui::MouseWheelUnit::Line => delta.y,
                egui::MouseWheelUnit::Point => delta.y / 40.0,
                egui::MouseWheelUnit::Page => delta.y * 3.0,
            },
            _ => 0.0,
        })
        .sum()
}

fn fly_key(key: Key) -> Option<FlyKey> {
    match key {
        Key::W => Some(FlyKey::W),
        Key::S => Some(FlyKey::S),
        Key::A => Some(FlyKey::A),
        Key::D => Some(FlyKey::D),
        Key::E => Some(FlyKey::E),
        Key::Q => Some(FlyKey::Q),
        _ => None,
    }
}

/// The labels the frame found visible, in their colours over the theme's label shadow. A label is
/// laid out once, without a colour of its own, and painted twice.
fn draw_labels(painter: &egui::Painter, rect: Rect, info: &gz_render::FrameInfo, ppp: f32, shadow: Color32) {
    let font = Font::Small.id();
    for l in info.labels.iter().filter(|l| l.visible) {
        let at = pos2(rect.min.x + l.position[0] / ppp, rect.min.y + l.position[1] / ppp);
        if !rect.contains(at) {
            continue;
        }
        let c = Color32::from_rgba_unmultiplied(l.color.0[0], l.color.0[1], l.color.0[2], 255);
        // a label is a name (data), except the one kind the editor names in its own words
        let shown = if l.text == crate::draw::PLAYER_START_LABEL {
            gz_ui::i18n::t(painter.ctx(), crate::draw::PLAYER_START_LABEL)
        } else {
            l.text.to_string()
        };
        let text = painter.layout_no_wrap(shown, font.clone(), Color32::PLACEHOLDER);
        // the anchor is the label's left edge at mid-height
        let corner = pos2(at.x, at.y - text.size().y * 0.5);
        painter.galley(corner + vec2(1.0, 1.0), text.clone(), shadow);
        painter.galley(corner, text, c);
    }
}

/// The frame the game's 4:3 screen shows, the rest dimmed.
fn draw_safe_rect(painter: &egui::Painter, rect: Rect, p: &Palette) {
    let (w, h) = (rect.width(), rect.height());
    let safe = if 0.75 * w > h {
        Rect::from_center_size(rect.center(), vec2(h * 4.0 / 3.0, h))
    } else {
        Rect::from_center_size(rect.center(), vec2(w, w * 0.75))
    };
    for r in [
        Rect::from_min_max(rect.min, pos2(safe.min.x, rect.max.y)),
        Rect::from_min_max(pos2(safe.max.x, rect.min.y), rect.max),
        Rect::from_min_max(pos2(safe.min.x, rect.min.y), pos2(safe.max.x, safe.min.y)),
        Rect::from_min_max(pos2(safe.min.x, safe.max.y), pos2(safe.max.x, rect.max.y)),
    ] {
        if r.width() > 0.5 && r.height() > 0.5 {
            painter.rect_filled(r, CornerRadius::ZERO, p.dim);
        }
    }
    painter.rect_stroke(safe, CornerRadius::ZERO, Stroke::new(1.0, p.accent_bright), StrokeKind::Inside);
}

/// Show > Stats: what the last pass drew, as a Mono pill under the left pills; as many lines as
/// the view has room for, and none on a view too narrow for them.
fn local_stats_pill(
    painter: &egui::Painter,
    rect: Rect,
    vp: &Viewport,
    info: Option<&gz_render::FrameInfo>,
    p: &Palette,
) {
    let rates = vp.rates.lock().clone();
    let stats = info.map(|i| i.stats).unwrap_or_default();
    let game = vp.read(|s| s.game.active().then_some((s.game.drawn.len(), s.game.in_view)));
    let (lights, of) = game.unwrap_or((stats.lights, stats.lights_in_view));
    let ctx = painter.ctx();
    let of_ = |a: usize, b: usize| tr!("{a} of {b}", a = a, b = b).text(ctx);
    let rows = [
        (gz_ui::i18n::t(ctx, "FPS"), format!("{:.0}", rates.ui_fps)),
        (gz_ui::i18n::t(ctx, "Render FPS"), format!("{:.0}", rates.render_fps)),
        (gz_ui::i18n::t(ctx, "Triangles"), stats.triangles.to_string()),
        (gz_ui::i18n::t(ctx, "Objects"), of_(stats.visible, stats.instances)),
        (gz_ui::i18n::t(ctx, "Draw calls"), stats.draws.to_string()),
        (gz_ui::i18n::t(ctx, "Lights"), of_(lights, of)),
    ];
    // the values line up in one column whatever the language's words are
    let pad_to = rows.iter().map(|(l, _)| l.chars().count()).max().unwrap_or(0) + 2;
    let lines = rows.map(|(l, v)| format!("{l:<pad_to$}{v}"));
    let font = Font::Mono.id();
    let line = 15.0;
    let pad = 8.0;
    let at = pos2(rect.min.x + 8.0, rect.min.y + 44.0);
    let n = (((rect.max.y - 8.0 - at.y - pad * 2.0) / line).floor().max(0.0) as usize).min(lines.len());
    let w = lines.iter().take(n).map(|l| text::width(ctx, &font, Tx::raw(l))).fold(0.0f32, f32::max);
    if n == 0 || w + pad * 2.0 > rect.width() - 16.0 {
        return;
    }
    let bg = Rect::from_min_size(at, vec2(w + pad * 2.0, n as f32 * line + pad * 1.5));
    painter.rect_filled(bg, CornerRadius::same(6), p.overlay.gamma_multiply(0.92));
    for (i, l) in lines.iter().take(n).enumerate() {
        painter.text(
            pos2(at.x + pad, at.y + pad * 0.75 + i as f32 * line),
            Align2::LEFT_TOP,
            l,
            font.clone(),
            p.text_secondary,
        );
    }
}

// ---- what the clicks do ---------------------------------------------------------------------

/// What a click may pick: a visible object of a visible kind; a member of a group that is not
/// editable by itself stands for its group.
fn accept(o: &Object, state: &gz_scene::SceneState) -> bool {
    o.flags.visible && state.tools.part(o.class).visible && (o.owner.is_none() || o.editable())
}

/// A click of Select: Ctrl adds to the selection, toggling the object hit; without it the click
/// replaces the selection. Any kind is picked. A way path's point under the click is selected
/// with its way, so the gizmo moves that point.
fn click_select(editor: &Arc<Editor>, vp: &Arc<Viewport>, ray: Ray, b: Buttons) {
    let Ok(svc) = SceneService::of(editor) else { return };
    let far = vp.read(|s| s.nav.far);
    let ed = editor.clone();
    let origin = ed.ui_origin();
    crate::gesture::spawn(editor, async move {
        if !b.ctrl {
            let _ = svc
                .store
                .send(SceneCmd::SelectAll { classes: None, select: false, invert: false }, origin)
                .await;
        }
        let (snap, pick) = svc.pick().await;
        let lib = svc.library.clone();
        let s = snap.clone();
        let hit = ed
            .exec
            .cpu(move || pick.ray_pick(&s, &lib, ray.origin, ray.dir, far, &|o: &Object| accept(o, &s)))
            .await;
        let Some(hit) = hit else { return };
        let Some(obj) = snap.get(hit.id) else { return };
        let id = match obj.owner {
            Some(owner) if !obj.editable() => owner,
            _ => obj.id,
        };
        let mode = if b.ctrl { SelectMode::Toggle } else { SelectMode::Add };
        let _ = svc.store.send(SceneCmd::Select { ids: vec![id], mode }, origin).await;
        if let ObjData::Way(w) = &obj.data
            && let Some((point, _)) = gz_scene::objtools::way::pick_point(w, ray.origin, ray.dir, far)
        {
            let edit = ToolEdit::SelectPoints {
                way: obj.id,
                points: vec![point],
                mode: if b.ctrl { SelectMode::Toggle } else { SelectMode::Set },
            };
            let _ = svc.store.send(SceneCmd::ObjTool(Box::new(edit)), origin).await;
        }
        ed.exec.wake().ping();
    });
}

/// The end of a marquee: what the box holds of any kind is added.
fn marquee_select(editor: &Arc<Editor>, vp: &Arc<Viewport>, size: [u32; 2], a: G2, b: G2) {
    let Ok(svc) = SceneService::of(editor) else { return };
    let (cam, prefs, far) = vp.read(|s| (s.nav.camera(), s.prefs, s.nav.far));
    let ed = editor.clone();
    crate::gesture::spawn(editor, async move {
        let (snap, pick) = svc.pick().await;
        let lib = svc.library.clone();
        let ids = ed
            .exec
            .cpu(move || {
                let depth = prefs.bp_lim_depth.then(|| {
                    crate::draw::box_depth(
                        &snap,
                        &lib,
                        &pick,
                        &cam,
                        size,
                        (a, b),
                        (prefs.near, far, prefs.bp_depth_tolerance),
                    )
                });
                crate::draw::frustum_select(&snap, &lib, &cam, size, a, b, None, depth.flatten())
            })
            .await;
        if !ids.is_empty() {
            let _ = svc.store.send(SceneCmd::Select { ids, mode: SelectMode::Add }, ed.ui_origin()).await;
        }
        ed.exec.wake().ping();
    });
}

/// One mouse move of a gizmo drag: the held objects' new transforms, as one merged undo step.
#[allow(clippy::too_many_lines)]
fn apply_drag(vp: &Viewport, d: &mut Drag, ray: &Ray, now: Pos2, editor: &Arc<Editor>) {
    // Alt: nothing moves until the copies are there; then the drag holds them
    if let Some(slot) = &d.copies {
        let Some(ids) = slot.lock().clone() else { return };
        let snap = d.svc.store.snapshot();
        d.held = held_of(&d.svc, &snap, &ids);
        d.points.clear();
        d.copies = None;
    }
    let (cam, grid, angle_step, scale_step) = vp.read(|s| {
        (s.nav.camera(), s.snap(GizmoType::Move), s.snap(GizmoType::Rotate), s.snap(GizmoType::Scale))
    });
    let origin = editor.ui_origin();
    match d.kind {
        GizmoType::Move => {
            let Some(delta) = gizmo::move_delta(d.status, d.pivot, d.axes, cam.forward, &d.ray0, ray) else {
                return;
            };
            let pivot_to = gizmo::moved(d.pivot, delta, d.status, d.axes, d.space, grid);
            if !d.nodes.is_empty() {
                let shift = pivot_to - d.pivot;
                crate::bulktools::move_nodes(editor, &d.nodes, |p| p + shift, &d.merge, origin);
                return;
            }
            let mut items = Vec::with_capacity(d.held.len());
            for h in &d.held {
                // a way path with selected points: those points move
                if let Some((_, pts)) = d.points.iter().find(|(id, _)| *id == h.id) {
                    crate::objtools::move_points(
                        &d.svc,
                        &[(h.id, pts.clone())],
                        pivot_to - d.pivot,
                        &d.merge,
                        origin,
                    );
                    continue;
                }
                let at = if d.held.len() == 1 {
                    gizmo::moved(h.position, delta, d.status, d.axes, d.space, grid)
                } else {
                    h.position + (pivot_to - d.pivot)
                };
                items.push((h.id, TransformEdit::Position(at)));
            }
            post(&d.svc, items, "Move", &d.merge, origin);
        }
        GizmoType::Rotate => {
            let axis = match d.status {
                Status::Screen => -cam.forward,
                st => st.axis().map_or(Vec3::Y, |i| d.axes[i]),
            };
            let raw = if d.edge {
                let radius = d.shapes.screen_ring.map_or(80.0, |r| r * gizmo::RING / gizmo::SCREEN_RING);
                gizmo::edge_angle(
                    d.shapes.centre,
                    G2::new(d.press.x, d.press.y),
                    G2::new(now.x, now.y),
                    radius,
                )
            } else {
                gizmo::ring_angle(axis, d.pivot, &d.ray0, ray).unwrap_or(d.angle)
            };
            let angle = match angle_step {
                Some(step) => gizmo::snapto(raw.to_degrees(), step).to_radians(),
                None => raw,
            };
            if (angle - d.angle).abs() < 1e-6 && d.angle != 0.0 {
                return;
            }
            d.angle = angle;
            if !d.nodes.is_empty() {
                let about_x = d.status == Status::X;
                let step = angle - d.applied;
                d.applied = angle;
                crate::bulktools::rotate_nodes(editor, about_x, step, &d.merge, origin);
                return;
            }
            let several = d.held.len() > 1;
            let mut items = Vec::with_capacity(d.held.len() * 2);
            for h in &d.held {
                let (r, p) = gizmo::turned(h.rotation, h.position, d.pivot, axis, angle);
                items.push((h.id, TransformEdit::Rotation(r)));
                if several {
                    items.push((h.id, TransformEdit::Position(p)));
                }
            }
            post(&d.svc, items, "Rotate", &d.merge, origin);
        }
        GizmoType::Scale => {
            let f = gizmo::scale_factors(
                d.status,
                &d.shapes,
                G2::new(d.press.x, d.press.y),
                G2::new(now.x, now.y),
            );
            let items = d
                .held
                .iter()
                .filter(|h| !h.dynamic)
                .map(|h| (h.id, TransformEdit::Scale(gizmo::scaled(h.scale, f, h.glow, scale_step))))
                .collect();
            post(&d.svc, items, "Scale", &d.merge, origin);
        }
        GizmoType::None => {}
    }
}

fn post(
    svc: &SceneService,
    items: Vec<(ObjId, TransformEdit)>,
    label: &str,
    merge: &SmolStr,
    origin: gz_editor::Origin,
) {
    if items.is_empty() {
        return;
    }
    svc.store.post(
        SceneCmd::Transform { items, members: true, label: label.into(), merge: Some(merge.clone()) },
        origin,
    );
}

/// Where a thing put down at the pointer meets the scene: with `surface` on the scene objects
/// and spawn points under it (on the nearest vertex of the one hit while Snap to Vertices is
/// on), else on the grid's plane. The surface's normal comes with it.
pub(crate) async fn pick_ground(
    editor: &Arc<Editor>,
    svc: &Arc<SceneService>,
    ray: Ray,
    surface: bool,
    far: f32,
) -> Option<(Vec3, Vec3)> {
    if surface {
        let vertex = Viewport::of(editor).read(|s| s.prefs.snaps.vertex);
        let (snap, pick) = svc.pick().await;
        let lib = svc.library.clone();
        let found = editor
            .exec
            .cpu(move || {
                let hit = pick.ray_pick(&snap, &lib, ray.origin, ray.dir, far, &|o: &Object| {
                    matches!(o.class, ObjClass::SceneObject | ObjClass::SpawnPoint)
                        && o.flags.visible
                        && snap.tools.part(o.class).visible
                })?;
                let mut at = hit.point;
                if vertex {
                    let reach = Vec3::splat(1.0);
                    let near = gz_scene::math::Aabb::new(at - reach, at + reach);
                    let tris = gz_scene::pick::box_triangles(&snap, &lib, &[hit.id], &near);
                    if let Some(v) = tris.iter().flatten().min_by(|a, b| {
                        a.distance_squared(hit.point).total_cmp(&b.distance_squared(hit.point))
                    }) {
                        at = *v;
                    }
                }
                Some((at, hit.normal.unwrap_or(Vec3::Y)))
            })
            .await;
        if found.is_some() {
            return found;
        }
    }
    let t = ray.intersect_plane_y(0.0).filter(|t| *t > 0.0)?;
    Some((ray.at(t), Vec3::Y))
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, pos2, vec2};

    use super::{PressStep, held_delta, mode_index, press_step};
    use crate::camera::Buttons;
    use gz_scene::SceneMode;

    #[test]
    fn the_mode_control_shows_the_scenes_mode() {
        assert_eq!(mode_index(None), 0);
        assert_eq!(mode_index(Some(SceneMode::Select)), 0);
        assert_eq!(mode_index(Some(SceneMode::Place)), 1);
        // every painted layer is Paint
        for m in [SceneMode::AiMap, SceneMode::Details, SceneMode::Wallmarks] {
            assert_eq!(mode_index(Some(m)), 2);
        }
        assert_eq!(mode_index(Some(SceneMode::Zones)), 3);
        // and each segment runs the mode's action, the one bound to its key
        assert_eq!(crate::actions::MODES[mode_index(Some(SceneMode::Zones))], "viewport.mode_zones");
    }

    #[test]
    fn a_left_press_is_a_click_until_it_moves() {
        let start = pos2(100.0, 100.0);
        let held = Buttons { left: true, ..Buttons::default() };
        let up = Buttons::default();
        // a jitter under the threshold keeps the click
        assert_eq!(press_step(&held, start, pos2(103.0, 101.0), true), PressStep::Wait);
        assert_eq!(press_step(&up, start, pos2(103.0, 101.0), true), PressStep::Click);
        // past it the drag is the camera's, and the button up after it does nothing
        assert_eq!(press_step(&held, start, pos2(106.0, 100.0), true), PressStep::Camera);
        assert_eq!(press_step(&up, start, pos2(106.0, 100.0), true), PressStep::Nothing);
        // a press that waits for no click (a mode already acted) clicks nothing
        assert_eq!(press_step(&up, start, start, false), PressStep::Nothing);
        // Ctrl draws the marquee; another key only stops the click
        let ctrl = Buttons { ctrl: true, ..held };
        assert_eq!(press_step(&ctrl, start, pos2(110.0, 110.0), true), PressStep::Marquee);
        let shift = Buttons { shift: true, ..held };
        assert_eq!(press_step(&shift, start, pos2(110.0, 110.0), true), PressStep::Drag);
    }

    #[test]
    fn the_held_cursor_turns_the_camera_by_the_mouse_not_by_where_it_is() {
        let hold = pos2(400.0, 300.0);
        // the mouse's own motion wins: the cursor was warped back, its place says nothing
        assert_eq!(held_delta(Some(vec2(-12.0, 3.0)), Some(hold), hold, 1.5), vec2(-12.0, 3.0));
        // without it, how far the cursor went from where it is held, in pixels
        assert_eq!(held_delta(None, Some(pos2(410.0, 296.0)), hold, 1.5), vec2(15.0, -6.0));
        assert_eq!(held_delta(None, None::<Pos2>, hold, 1.5), vec2(0.0, 0.0));
    }
}
