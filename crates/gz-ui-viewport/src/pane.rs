//! [`ViewportPane`]: the picture, the pills, the gizmos and the mouse (`UIRenderForm`,
//! `UI_ViewportNavigation::Draw`, `DrawRenderToolBar`, `TUI::Mouse*`, the tools' controls).

use std::sync::Arc;
use std::time::Instant;

use egui::{
    Align, Align2, Color32, CornerRadius, Key, Layout, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder,
    Vec2, pos2, vec2,
};
use glam::{Vec2 as G2, Vec3};
use gz_app::{Chord, DocCx};
use gz_editor::{Editor, SmolStr, Value, json};
use gz_render::{Color, Ray};
use gz_scene::apply::{SelectMode, TransformEdit};
use gz_scene::objects::{ParticlesData, SceneObjectData, WayData, default_data};
use gz_scene::{ObjClass, ObjData, ObjId, Object, SceneCmd, SceneService};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Kind, Palette, Toolbar, ToolbarStyle, palette};

use crate::camera::{Buttons, FlyKey, Target};
use crate::gizmo::{self, GizmoType, Status};
use crate::keys;
use crate::menus;
use crate::service::{Action, Viewport};

/// One gesture of the mouse in the viewport.
enum Gesture {
    None,
    /// The camera took the press.
    Camera,
    /// A left press the tools did not capture: a drag that follows flies the camera, a Ctrl
    /// drag draws a marquee.
    Press {
        start: Pos2,
    },
    Transform(Box<Drag>),
    Marquee {
        start: Pos2,
        now: Pos2,
    },
    /// The Sector tool's mesh paint (Place with Meshes Add or Remove).
    ToolPaint(crate::objtools::Paint),
    /// The Sector tool's Box Pick.
    ToolBox {
        start: Pos2,
        now: Pos2,
    },
    /// The AI map's brush stroke (Place with the AI Map tool).
    AiPaint(crate::bulktools::Paint),
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

/// A drag of the gizmo (`TUI_CustomControl::SelectStart` .. `SelectEnd`).
struct Drag {
    kind: GizmoType,
    status: Status,
    ray0: Ray,
    start: Vec3,
    held: Vec<Held>,
    /// The rotation each object has now: a rotate drag turns them step by step.
    turned: Vec<Vec3>,
    remainder: f32,
    merge: SmolStr,
    svc: Arc<SceneService>,
    /// In the Way tool's Points mode: the selected points of each held way, as at the press.
    points: Vec<(ObjId, Vec<(usize, Vec3)>)>,
    /// With the AI Map tool: the selected nodes, as at the press.
    nodes: Vec<(u32, Vec3)>,
}

/// The 3D view of the Level tab. UI thread only.
pub struct ViewportPane {
    editor: Arc<Editor>,
    vp: Arc<Viewport>,
    texture: Option<(egui::TextureId, u64)>,
    gesture: Gesture,
    hover: Status,
    last: Option<Instant>,
    serial: u64,
    maximized: bool,
    maximize_asked: bool,
    /// The canvas as last laid out, points.
    rect: Rect,
}

impl ViewportPane {
    #[must_use]
    pub fn new(editor: &Arc<Editor>, gpu: &egui_wgpu::RenderState) -> Self {
        let vp = Viewport::of(editor);
        vp.ensure_renderer(editor, gpu);
        crate::bulktools::provide_axes(editor);
        Self {
            editor: editor.clone(),
            vp,
            texture: None,
            gesture: Gesture::None,
            hover: Status::None,
            last: None,
            serial: 0,
            maximized: false,
            maximize_asked: false,
            rect: Rect::NOTHING,
        }
    }

    /// Whether View Options' Maximize Viewport is checked (the Level tab's F11 state).
    pub fn set_maximized(&mut self, on: bool) {
        self.maximized = on;
    }

    /// Maximize Viewport was picked in View Options since the last ask: the Level tab toggles.
    pub fn take_maximize_request(&mut self) -> bool {
        std::mem::take(&mut self.maximize_asked)
    }

    /// Draws the viewport into the space it is given: the picture, the pills, the gizmo, input.
    pub fn ui(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let (rect, canvas) =
            ui.allocate_exact_size(ui.available_size().max(vec2(32.0, 32.0)), Sense::click_and_drag());
        self.rect = rect;
        let ppp = ctx.pixels_per_point();
        let size =
            [(rect.width() * ppp).round().max(32.0) as u32, (rect.height() * ppp).round().max(32.0) as u32];
        let renderer = self.vp.ensure_renderer(cx.editor, cx.gpu);
        let canvas_c = Color::rgb(p.canvas.r(), p.canvas.g(), p.canvas.b());
        if self.vp.read(|s| s.size != size || s.background != canvas_c) {
            self.vp.with(|s| {
                s.size = size;
                s.background = canvas_c;
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
        // View > Render > Quality: the picture, rendered at a scale of the pane's size, fills it;
        // a label's pixel of the picture is this many pixels of it
        let mut label_px = ppp * self.vp.read(|s| s.view.quality);
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
            draw_labels(&painter, rect, info, label_px);
        }
        let v = self.vp.read(|s| s.view.clone());
        if v.safe_rect {
            draw_safe_rect(&painter, rect, &p);
        }
        if let Gesture::Marquee { start, now } | Gesture::ToolBox { start, now } = &self.gesture {
            let r = Rect::from_two_pos(*start, *now);
            painter.rect_filled(r, CornerRadius::ZERO, p.accent_bright.gamma_multiply(0.12));
            painter.rect_stroke(r, CornerRadius::ZERO, Stroke::new(1.0, p.accent_bright), StrokeKind::Inside);
        }
        self.draw_gizmo(&painter, rect, size, ppp, &p);
        if let Some((text_, at)) = self.vp.read(|s| s.status.clone())
            && at.elapsed().as_secs_f32() < 2.0
        {
            text::paint(
                &painter,
                &Font::Small.id(),
                pos2(rect.min.x + 8.0, rect.max.y - 8.0),
                Align2::LEFT_BOTTOM,
                p.text_secondary,
                &text_,
                None,
                Clip::End,
            );
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
        if v.stats {
            local_stats_pill(&painter, rect, &self.vp, finished.as_ref(), &p);
        }
        self.local_axis_gizmo(ui, rect, &p);
        self.pills(ui, rect);
        if renderer.as_ref().is_some_and(|r| r.finished().stats.placeholders > 0) {
            // meshes still loading: look again soon
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// A tick while the Level tab is not drawn: the camera is kept, no scene pass runs.
    pub fn inactive(&mut self, cx: &mut DocCx<'_>) {
        self.vp.sync(cx.editor);
        self.gesture = Gesture::None;
    }

    /// The scene's key bindings, routed here by the shell only while Level owns the keys.
    pub fn shortcut(&mut self, cx: &mut DocCx<'_>, chord: &Chord) -> bool {
        // flying with the right button: WASD and QE are the camera's
        if self.vp.read(|s| s.nav.flying()) && fly_key(chord.key).is_some() {
            return true;
        }
        // `TUI::KeyDown`: Esc leaves placing before the command bound to it runs
        if chord.key == egui::Key::Escape && !chord.ctrl && !chord.shift && !chord.alt {
            self.vp.with(|s| s.tool.action = crate::service::Action::Select);
        }
        match keys::act_of(cx.editor, chord) {
            Some(act) => {
                keys::run(cx.editor, act);
                true
            }
            None => false,
        }
    }

    /// What `state` / `ui_list` answer about it.
    #[must_use]
    pub fn mcp_state(&self) -> Value {
        let mut out = crate::cmd::view_state(&self.vp);
        if let Value::Object(m) = &mut out {
            let ppp_rect = self.rect;
            m.insert(
                "rect".into(),
                json!([ppp_rect.min.x, ppp_rect.min.y, ppp_rect.width(), ppp_rect.height()]),
            );
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

    fn input(&mut self, ctx: &egui::Context, canvas: &egui::Response, rect: Rect, ppp: f32, size: [u32; 2]) {
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
        let over = canvas.hovered();
        let pos = pos.unwrap_or(rect.center());
        // C++: a pointer outside is pinned to the canvas's edge
        let clamped = pos2(pos.x.clamp(rect.min.x, rect.max.x), pos.y.clamp(rect.min.y, rect.max.y));
        let px = G2::new((clamped.x - rect.min.x) * ppp, (clamped.y - rect.min.y) * ppp);
        let (dx, dy) = (delta.x * ppp, delta.y * ppp);
        let ray = self.vp.read(|s| s.nav.ray(size, px));

        // the gizmo part under the mouse
        if matches!(self.gesture, Gesture::None) {
            self.hover = if over { self.hover_status(ray.as_ref(), size) } else { Status::None };
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
                if pressed && over && b.any_button() {
                    if self.vp.with(|s| s.nav.move_start(b)) {
                        self.gesture = Gesture::Camera;
                    } else if b.left
                        && let Some(ray) = ray
                    {
                        self.tool_press(&b, ray, clamped);
                    }
                }
            }
            Gesture::Camera => {
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
            Gesture::Press { start } => {
                let start = *start;
                if !b.left {
                    self.gesture = Gesture::None;
                } else if b.ctrl && (clamped - start).length() > 4.0 {
                    self.gesture = Gesture::Marquee { start, now: clamped };
                } else if (dx != 0.0 || dy != 0.0) && !(b.right || b.middle || b.alt || b.shift || b.ctrl) {
                    // a left drag no tool took is the camera's
                    if self.vp.with(|s| s.nav.left_drag_start(b)) {
                        self.gesture = Gesture::Camera;
                    }
                }
            }
            Gesture::Marquee { start, now } => {
                *now = clamped;
                if !b.left {
                    let (a, z) = (*start, *now);
                    self.gesture = Gesture::None;
                    let to_px = |q: Pos2| G2::new((q.x - rect.min.x) * ppp, (q.y - rect.min.y) * ppp);
                    let target = scene_target(&self.editor).unwrap_or(ObjClass::SceneObject);
                    if crate::bulktools::marquee(&self.editor, &self.vp, size, to_px(a), to_px(z), target) {
                    } else if crate::objtools::way_points_mode(&self.editor) {
                        crate::objtools::way_points_marquee(&self.editor, &self.vp, size, to_px(a), to_px(z));
                    } else {
                        marquee_select(&self.editor, &self.vp, size, to_px(a), to_px(z));
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
                    crate::bulktools::paint_end(&self.editor, &self.vp, b);
                    self.gesture = Gesture::None;
                } else if (dx != 0.0 || dy != 0.0)
                    && let Some(ray) = ray
                {
                    crate::bulktools::ai_step(&self.editor, &self.vp, paint, ray, false, false);
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
                    apply_drag(&self.vp, drag, &ray, dx, dy, &self.editor);
                }
            }
        }

        // flying: WASD and QE held move the camera every frame
        let flying = self.vp.read(|s| s.nav.flying());
        if flying {
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
        self.last = Some(Instant::now());
    }

    /// The gizmo's part under the ray (`Gizmo::OnFrame`), when the gizmo is up.
    fn hover_status(&self, ray: Option<&Ray>, size: [u32; 2]) -> Status {
        let Some(ray) = ray else { return Status::None };
        let (kind, action, anchor, cam, class) = self
            .vp
            .read(|s| (s.tool.gizmo, s.tool.action, s.anchor, s.nav.camera(), scene_target(&self.editor)));
        if action != Action::Select || kind == GizmoType::None {
            return Status::None;
        }
        let Some(anchor) = anchor else { return Status::None };
        let Some(at) = gizmo::screen_position(&cam, size, anchor.position) else { return Status::None };
        gizmo::hover(kind, at, &anchor.rotation, ray, class == Some(ObjClass::AiMap))
    }

    /// A left press the camera did not take (`TUI_CustomControl::Start`).
    fn tool_press(&mut self, b: &Buttons, ray: Ray, at: Pos2) {
        let (action, kind) = self.vp.read(|s| (s.tool.action, s.tool.gizmo));
        if action == Action::Place {
            self.serial += 1;
            match crate::bulktools::place_press(&self.editor, &self.vp, ray, *b, self.serial) {
                crate::bulktools::BulkGesture::Paint(p) => {
                    self.gesture = Gesture::AiPaint(p);
                    return;
                }
                crate::bulktools::BulkGesture::Done => {
                    self.gesture = Gesture::Press { start: at };
                    return;
                }
                crate::bulktools::BulkGesture::None => {}
            }
            match crate::objtools::place_press(&self.editor, &self.vp, ray, *b, self.serial) {
                crate::objtools::ToolGesture::Paint(p) => self.gesture = Gesture::ToolPaint(p),
                crate::objtools::ToolGesture::BoxPick => self.gesture = Gesture::ToolBox { start: at, now: at },
                crate::objtools::ToolGesture::Done => self.gesture = Gesture::Press { start: at },
                crate::objtools::ToolGesture::None => {
                    place(&self.editor, &self.vp, ray, *b);
                    self.gesture = Gesture::Press { start: at };
                }
            }
            return;
        }
        let target = scene_target(&self.editor).unwrap_or(ObjClass::SceneObject);
        let sup = gizmo::supports(target);
        let supported = match kind {
            GizmoType::Move => sup[0],
            GizmoType::Rotate => sup[1],
            GizmoType::Scale => sup[2],
            GizmoType::None => false,
        };
        if supported
            && self.hover != Status::None
            && let Some(drag) = self.start_drag(kind, ray, target)
        {
            self.gesture = Gesture::Transform(Box::new(drag));
            return;
        }
        if crate::bulktools::click(&self.editor, &self.vp, ray, *b, target) {
        } else if target == ObjClass::Way && crate::objtools::way_points_mode(&self.editor) {
            crate::objtools::way_point_click(&self.editor, &self.vp, ray, *b);
        } else {
            click_select(&self.editor, &self.vp, ray, *b, target);
        }
        self.gesture = Gesture::Press { start: at };
    }

    fn start_drag(&mut self, kind: GizmoType, ray0: Ray, class: ObjClass) -> Option<Drag> {
        let svc = SceneService::of(&self.editor).ok()?;
        let snap = svc.store.snapshot();
        let start = self.vp.read(|s| s.anchor.map(|a| a.position))?;
        if class == ObjClass::AiMap {
            let nodes = crate::bulktools::held_nodes(&self.editor);
            if nodes.is_empty() {
                return None;
            }
            self.serial += 1;
            return Some(Drag {
                kind,
                status: self.hover,
                ray0,
                start,
                held: Vec::new(),
                turned: Vec::new(),
                remainder: 0.0,
                merge: SmolStr::new(format!("viewport.aimap.gizmo.{}", self.serial)),
                svc,
                points: Vec::new(),
                nodes,
            });
        }
        let held: Vec<Held> = snap
            .of_class(class)
            .filter(|o| snap.is_selected(o.id) && o.flags.visible && o.editable())
            .map(|o| Held {
                id: o.id,
                position: o.position(),
                rotation: o.rotation(),
                scale: o.scale(),
                glow: o.class == ObjClass::Glow,
                dynamic: matches!(&o.data, ObjData::SceneObject(d) if svc.library.cached(&d.reference).is_some_and(|l| l.is_dynamic())),
            })
            .collect();
        if held.is_empty() {
            return None;
        }
        self.serial += 1;
        let turned = held.iter().map(|h| h.rotation).collect();
        let points = if class == ObjClass::Way && crate::objtools::way_points_mode(&self.editor) {
            crate::objtools::held_points(&self.editor, &held.iter().map(|h| h.id).collect::<Vec<_>>())
        } else {
            Vec::new()
        };
        Some(Drag {
            kind,
            status: self.hover,
            ray0,
            start,
            held,
            turned,
            remainder: 0.0,
            merge: SmolStr::new(format!("viewport.gizmo.{}", self.serial)),
            svc,
            points,
            nodes: Vec::new(),
        })
    }

    // ---- what is drawn over the picture --------------------------------------------------------

    fn draw_gizmo(&self, painter: &egui::Painter, rect: Rect, size: [u32; 2], ppp: f32, p: &Palette) {
        let (kind, action, anchor, cam) =
            self.vp.read(|s| (s.tool.gizmo, s.tool.action, s.anchor, s.nav.camera()));
        if action != Action::Select || kind == GizmoType::None {
            return;
        }
        let Some(anchor) = anchor else { return };
        let Some(at) = gizmo::screen_position(&cam, size, anchor.position) else { return };
        let vp_m = cam.view_proj(size);
        let to_screen = |w: Vec3| {
            gz_render::project(&vp_m, size, w).map(|q| pos2(rect.min.x + q.x / ppp, rect.min.y + q.y / ppp))
        };
        let status = match &self.gesture {
            Gesture::Transform(d) => d.status,
            _ => self.hover,
        };
        let aimap = scene_target(&self.editor) == Some(ObjClass::AiMap);
        let axes =
            [(Vec3::X, p.axis_x, Status::X), (Vec3::Y, p.axis_y, Status::Y), (Vec3::Z, p.axis_z, Status::Z)];
        let hot = |s: Status, c: Color32| if status == s { p.text } else { c };
        let Some(o) = to_screen(at) else { return };
        match kind {
            GizmoType::Move => {
                for (axis, c, s) in axes {
                    if aimap && s != Status::Y {
                        continue;
                    }
                    let c = hot(s, c);
                    let (Some(shaft), Some(tip)) = (to_screen(at + axis * 0.82), to_screen(at + axis)) else {
                        continue;
                    };
                    painter.line_segment([o, shaft], Stroke::new(3.0, c));
                    let dir = (tip - shaft).normalized();
                    let side = vec2(-dir.y, dir.x) * ((tip - shaft).length() * 0.35).clamp(3.0, 7.0);
                    painter.add(egui::Shape::convex_polygon(
                        vec![tip, shaft + side, shaft - side],
                        c,
                        Stroke::NONE,
                    ));
                }
                let pivot = if status == Status::Xyz { p.text } else { p.text_secondary };
                painter.rect_filled(
                    Rect::from_center_size(o, Vec2::splat(6.0)),
                    CornerRadius::same(1),
                    pivot,
                );
            }
            GizmoType::Scale => {
                for (axis, c, s) in axes {
                    let c = hot(s, c);
                    let Some(end) = to_screen(at + axis) else { continue };
                    painter.line_segment([o, end], Stroke::new(2.0, c));
                    painter.rect_filled(
                        Rect::from_center_size(end, Vec2::splat(7.0)),
                        CornerRadius::same(1),
                        c,
                    );
                }
                let centre = if status == Status::Xyz { p.text } else { p.text_secondary };
                painter.rect_filled(
                    Rect::from_center_size(o, Vec2::splat(9.0)),
                    CornerRadius::same(1),
                    centre,
                );
            }
            GizmoType::Rotate => {
                let [x, y, z] = gizmo::rotate_axes(&anchor.rotation);
                for (axis, c, s) in
                    [(x, p.axis_x, Status::X), (y, p.axis_y, Status::Y), (z, p.axis_z, Status::Z)]
                {
                    if aimap && s == Status::Y {
                        continue;
                    }
                    let c = hot(s, c);
                    let u = axis.any_orthonormal_vector();
                    let w = axis.cross(u);
                    let ring: Vec<Pos2> = (0..=48)
                        .filter_map(|i| {
                            let t = i as f32 / 48.0 * std::f32::consts::TAU;
                            to_screen(at + (u * t.cos() + w * t.sin()))
                        })
                        .collect();
                    if ring.len() > 2 {
                        painter.add(egui::Shape::line(
                            ring,
                            Stroke::new(if status == s { 3.0 } else { 2.0 }, c),
                        ));
                    }
                }
                painter.circle_filled(o, 2.5, p.text_secondary);
            }
            GizmoType::None => {}
        }
    }

    /// The 64 pt clickable axis gizmo at the top right (`ViewportNavigation::Draw`); hidden
    /// under 480 pt and when the view is too short for it.
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
        let tip = if target == Target::Perspective {
            "Perspective: back to the free view".to_owned()
        } else {
            format!("{} view", target.name())
        };
        gz_ui::audit::control(ui.ctx(), "Axis gizmo", true, None, Some(&tip), r);
        if resp.on_hover_text(tip).clicked() {
            self.vp.with(|s| s.nav.set_target(target));
        }
    }

    /// The pills (`DrawRenderToolBar`): the view options, the camera view and what is shown on
    /// the left; the transform tool, object snap and the grid, scale and angle snaps on the
    /// right. One overlay toolbar, so a narrow viewport folds the snaps into its overflow.
    fn pills(&mut self, ui: &mut Ui, rect: Rect) {
        let ed = self.editor.clone();
        let vp = self.vp.clone();
        let target = scene_target(&ed).unwrap_or(ObjClass::SceneObject);
        let [can_move, can_rotate, can_scale] = gizmo::supports(target);
        // a transform the tool cannot do falls back to plain selection
        let reset = vp.read(|s| {
            s.tool.action == Action::Select
                && ((s.tool.gizmo == GizmoType::Move && !can_move)
                    || (s.tool.gizmo == GizmoType::Rotate && !can_rotate)
                    || (s.tool.gizmo == GizmoType::Scale && !can_scale))
        });
        if reset {
            vp.with(|s| s.tool.gizmo = GizmoType::None);
        }
        let (tool, settings, view_target, ortho) =
            vp.read(|s| (s.tool, s.settings, s.nav.current(), s.nav.ortho));
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
        let mut maximize = false;
        let maximized = self.maximized;
        {
            let mut tb = Toolbar::begin(&mut child, "viewport", ToolbarStyle::Overlay);
            tb.group(3);
            tb.menu("options", Icon::ViewOptions, "", "View options", Some("viewport:options"), true, |ui| {
                menus::options_body(ui, &ed);
                ui.separator();
                if gz_ui::menu_item(
                    ui,
                    "Maximize Viewport",
                    gz_ui::MenuItem { keys: "F11", selected: maximized, ..gz_ui::MenuItem::default() },
                ) {
                    maximize = true;
                }
            });
            let icon =
                if view_target == Target::Perspective && !ortho { Icon::Perspective } else { Icon::Ortho };
            tb.menu(
                "perspective",
                icon,
                view_target.name(),
                "Camera view, Frame All and Frame Selection",
                Some("viewport:perspective"),
                true,
                |ui| menus::perspective_body(ui, &ed),
            );
            tb.menu(
                "show",
                Icon::Eye,
                "Show",
                "What the viewport draws over the scene",
                Some("viewport:show"),
                true,
                |ui| menus::show_body(ui, &ed),
            );
            tb.spring();

            tb.group(2);
            let title = tool_title(target);
            if tb.action(
                "tool_select",
                Icon::Select,
                "",
                "Select",
                true,
                tool.action == Action::Select && tool.gizmo == GizmoType::None,
                Kind::Ghost,
            ) {
                keys::run(&ed, keys::Act::Gizmo(GizmoType::None));
            }
            if tb.action(
                "tool_place",
                Icon::Place,
                "",
                "Place: a click in the viewport adds what the Tools panel has picked",
                true,
                tool.action == Action::Place,
                Kind::Ghost,
            ) {
                vp.with(|s| {
                    s.tool.action = Action::Place;
                    s.tool.gizmo = GizmoType::None;
                });
            }
            for (id, icon, tip, cannot, kind, on) in [
                (
                    "tool_move",
                    Icon::Move,
                    "Move the selection",
                    "cannot move objects",
                    GizmoType::Move,
                    can_move,
                ),
                (
                    "tool_rotate",
                    Icon::Rotate,
                    "Rotate the selection",
                    "cannot rotate objects",
                    GizmoType::Rotate,
                    can_rotate,
                ),
                (
                    "tool_scale",
                    Icon::Scale,
                    "Scale the selection",
                    "cannot scale objects",
                    GizmoType::Scale,
                    can_scale,
                ),
            ] {
                let tip = if on { tip.to_owned() } else { format!("The {title} tool {cannot}") };
                if tb.action(
                    id,
                    icon,
                    "",
                    &tip,
                    on,
                    tool.action == Action::Select && tool.gizmo == kind,
                    Kind::Ghost,
                ) {
                    keys::run(&ed, keys::Act::Gizmo(kind));
                }
            }

            tb.group(1);
            if tb.action(
                "osnap",
                Icon::SnapObject,
                "",
                "Object Snap: placed and moved objects land on other objects",
                true,
                settings.object_snap,
                Kind::Ghost,
            ) {
                vp.with(|s| s.settings.object_snap = !s.settings.object_snap);
            }
            tb.menu("snap", Icon::None, "", "Snap options", Some("viewport:snap"), true, |ui| {
                menus::snap_body(ui, &ed)
            });

            for (kind, icon, toggle, name, tip, menu, request) in [
                (
                    GizmoType::Move,
                    Icon::SnapGrid,
                    "grid_snap",
                    "Grid",
                    "Grid Snap: movement goes in steps",
                    "grid",
                    "viewport:grid",
                ),
                (
                    GizmoType::Scale,
                    Icon::SnapScale,
                    "scale_snap",
                    "Scale",
                    "Scale Snap: scaling goes in steps",
                    "scale",
                    "viewport:scale",
                ),
                (
                    GizmoType::Rotate,
                    Icon::SnapAngle,
                    "angle_snap",
                    "Angle",
                    "Angle Snap: rotation goes in steps",
                    "angle",
                    "viewport:angle",
                ),
            ] {
                tb.group(0);
                let (on, step) = vp.read(|s| (s.snap_on(kind), s.step(kind)));
                if tb.action(toggle, icon, "", tip, true, on, Kind::Ghost) {
                    vp.with(|s| match kind {
                        GizmoType::Move => s.tool.grid_snap = !s.tool.grid_snap,
                        GizmoType::Scale => s.tool.scale_snap = !s.tool.scale_snap,
                        _ => s.tool.angle_snap = !s.tool.angle_snap,
                    });
                }
                let value = menus::step_label(kind, step);
                tb.menu(menu, Icon::None, &value, &format!("{name} step"), Some(request), true, |ui| {
                    menus::step_body(ui, &ed, kind)
                });
            }
        }
        if maximize {
            self.maximize_asked = true;
        }
    }
}

/// The class the tools work on now (`LTools->CurrentClassID()`).
fn scene_target(editor: &Editor) -> Option<ObjClass> {
    editor
        .project_opt()
        .and_then(|s| s.services.get::<SceneService>())
        .map(|svc| svc.store.snapshot().tools.target)
}

/// The Tools panel's title of a class (`kToolClasses`).
#[must_use]
pub fn tool_title(class: ObjClass) -> &'static str {
    match class {
        ObjClass::SceneObject => "Object",
        ObjClass::Light => "Light",
        ObjClass::SoundSrc => "Sound Source",
        ObjClass::SoundEnv => "Sound Environment",
        ObjClass::Glow => "Glow",
        ObjClass::Shape => "Shape",
        ObjClass::SpawnPoint => "Spawn",
        ObjClass::Way => "Way",
        ObjClass::Sector => "Sector",
        ObjClass::Portal => "Portal",
        ObjClass::Group => "Group",
        ObjClass::Particles => "Particles",
        ObjClass::DetailObjects => "Detail Objects",
        ObjClass::AiMap => "AI Map",
        ObjClass::Wallmark => "Wallmark",
        ObjClass::FogVolume => "Fog Volume",
    }
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

/// The labels the frame found visible (`OutText`), white on a black shadow.
fn draw_labels(painter: &egui::Painter, rect: Rect, info: &gz_render::FrameInfo, ppp: f32) {
    let font = Font::Small.id();
    for l in info.labels.iter().filter(|l| l.visible) {
        let at = pos2(rect.min.x + l.position[0] / ppp, rect.min.y + l.position[1] / ppp);
        if !rect.contains(at) {
            continue;
        }
        let c = Color32::from_rgba_unmultiplied(l.color.0[0], l.color.0[1], l.color.0[2], 255);
        painter.text(at + vec2(1.0, 1.0), Align2::LEFT_CENTER, l.text.as_str(), font.clone(), Color32::BLACK);
        painter.text(at, Align2::LEFT_CENTER, l.text.as_str(), font.clone(), c);
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
    let dim = p.dim;
    for r in [
        Rect::from_min_max(rect.min, pos2(safe.min.x, rect.max.y)),
        Rect::from_min_max(pos2(safe.max.x, rect.min.y), rect.max),
        Rect::from_min_max(pos2(safe.min.x, rect.min.y), pos2(safe.max.x, safe.min.y)),
        Rect::from_min_max(pos2(safe.min.x, safe.max.y), pos2(safe.max.x, rect.max.y)),
    ] {
        if r.width() > 0.5 && r.height() > 0.5 {
            painter.rect_filled(r, CornerRadius::ZERO, dim);
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
    let lines = [
        format!("FPS         {:.0}", rates.ui_fps),
        format!("Render FPS  {:.0}", rates.render_fps),
        format!("Triangles   {}", stats.triangles),
        format!("Objects     {} of {}", stats.visible, stats.instances),
        format!("Draw calls  {}", stats.draws),
        format!("Lights      {lights} of {of}"),
    ];
    let font = Font::Mono.id();
    let line = 15.0;
    let pad = 8.0;
    let at = pos2(rect.min.x + 8.0, rect.min.y + 44.0);
    let n = (((rect.max.y - 8.0 - at.y - pad * 2.0) / line).floor().max(0.0) as usize).min(lines.len());
    let w = lines.iter().take(n).map(|l| text::width(painter.ctx(), &font, l)).fold(0.0f32, f32::max);
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

// ---- what the tools do --------------------------------------------------------------------------

/// The pick the C++ selection makes (`RaySelect`): visible objects of a visible tool, of the
/// current class or of any class; an object inside a group that is not editable picks the group.
fn accept(o: &Object, state: &gz_scene::SceneState, filter: Option<ObjClass>) -> bool {
    o.flags.visible
        && state.tools.part(o.class).visible
        && filter.is_none_or(|c| c == o.class)
        && (o.owner.is_none() || o.editable())
}

/// A click of the selection tool (`SelectStart`): Ctrl adds to what is selected, toggling the
/// object hit; without it the click replaces the selection. Picking any class makes the class
/// hit the tools' target.
fn click_select(editor: &Arc<Editor>, vp: &Arc<Viewport>, ray: Ray, b: Buttons, target: ObjClass) {
    let Ok(svc) = SceneService::of(editor) else { return };
    let any = vp.read(|s| s.settings.pick_any_class);
    let far = vp.read(|s| s.nav.far);
    let ed = editor.clone();
    let origin = ed.ui_origin();
    editor.exec.spawn(async move {
        if !b.ctrl {
            let _ = svc
                .store
                .send(
                    SceneCmd::SelectClass { class: if any { None } else { Some(target) }, select: false },
                    origin,
                )
                .await;
        }
        let (snap, pick) = svc.pick().await;
        let lib = svc.library.clone();
        let filter = if any { None } else { Some(target) };
        let s = snap.clone();
        let hit = ed
            .exec
            .cpu(move || {
                pick.ray_pick(&s, &lib, ray.origin, ray.dir, far, &|o: &Object| accept(o, &s, filter))
            })
            .await;
        let Some(hit) = hit else { return };
        let Some(obj) = snap.get(hit.id) else { return };
        let id = match obj.owner {
            Some(owner) if !obj.editable() => owner,
            _ => obj.id,
        };
        let class = snap.get(id).map_or(obj.class, |o| o.class);
        let _ = svc
            .store
            .send(
                SceneCmd::Select {
                    ids: vec![id],
                    mode: if b.ctrl { SelectMode::Toggle } else { SelectMode::Add },
                },
                origin,
            )
            .await;
        if any && class != target {
            let _ = svc.store.send(SceneCmd::SetTarget(class), origin).await;
        }
        ed.exec.wake().ping();
    });
}

/// The end of a marquee (`FrustumSelect`): what the box holds of the current class is added.
fn marquee_select(editor: &Arc<Editor>, vp: &Arc<Viewport>, size: [u32; 2], a: G2, b: G2) {
    let Ok(svc) = SceneService::of(editor) else { return };
    let cam = vp.read(|s| s.nav.camera());
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let snap = svc.store.snapshot();
        let lib = svc.library.clone();
        let class = snap.tools.target;
        let ids =
            ed.exec.cpu(move || crate::draw::frustum_select(&snap, &lib, &cam, size, a, b, class)).await;
        if !ids.is_empty() {
            let _ = svc.store.send(SceneCmd::Select { ids, mode: SelectMode::Add }, ed.ui_origin()).await;
        }
        ed.exec.wake().ping();
    });
}

/// One mouse move of a gizmo drag: the held objects' new transforms, as one merged undo step.
fn apply_drag(vp: &Viewport, d: &mut Drag, ray: &Ray, dx: f32, dy: f32, editor: &Arc<Editor>) {
    let (cam, grid, scale_step, angle, sr) = vp.read(|s| {
        (
            s.nav.camera(),
            s.snap_on(GizmoType::Move).then_some(s.step(GizmoType::Move)),
            s.snap_on(GizmoType::Scale).then_some(s.step(GizmoType::Scale)),
            s.snap_on(GizmoType::Rotate).then_some(s.step(GizmoType::Rotate)),
            s.prefs.tools_sr(),
        )
    });
    // the AI map's nodes (`TUI_ControlAIMapNodeSelect`)
    if !d.nodes.is_empty() {
        match d.kind {
            GizmoType::Move => {
                if let Some((delta, vector)) = gizmo::move_delta(d.status, d.start, cam.forward, &d.ray0, ray) {
                    crate::bulktools::move_nodes(editor, &d.nodes, |p| gizmo::moved(p, delta, vector, grid), &d.merge, editor.ui_origin());
                }
            }
            GizmoType::Rotate => {
                let amount = gizmo::rotate_amount(d.status, dx, dy, sr, angle, &mut d.remainder);
                if amount != 0.0 {
                    let about_x = gizmo::rotate_vector(d.status).x.abs() > gz_scene::math::EPS;
                    crate::bulktools::rotate_nodes(editor, about_x, amount, &d.merge, editor.ui_origin());
                }
            }
            _ => {}
        }
        return;
    }
    let mut items = Vec::with_capacity(d.held.len());
    let label = match d.kind {
        GizmoType::Move => {
            let Some((delta, vector)) = gizmo::move_delta(d.status, d.start, cam.forward, &d.ray0, ray)
            else {
                return;
            };
            for h in &d.held {
                let at = gizmo::moved(h.position, delta, vector, grid);
                // Points mode: only the selected points of the way move
                if let Some((_, pts)) = d.points.iter().find(|(id, _)| *id == h.id) {
                    crate::objtools::move_points(&d.svc, &[(h.id, pts.clone())], at - h.position, &d.merge, editor.ui_origin());
                    continue;
                }
                items.push((h.id, TransformEdit::Position(at)));
            }
            "Move"
        }
        GizmoType::Scale => {
            let Some((delta, vector)) = gizmo::scale_delta(d.status, d.start, &cam, &d.ray0, ray) else {
                return;
            };
            for h in &d.held {
                if h.dynamic {
                    continue;
                }
                items.push((
                    h.id,
                    TransformEdit::Scale(gizmo::scaled(h.scale, delta, vector, d.status, h.glow, scale_step)),
                ));
            }
            "Scale"
        }
        GizmoType::Rotate => {
            let amount = gizmo::rotate_amount(d.status, dx, dy, sr, angle, &mut d.remainder);
            if amount == 0.0 {
                return;
            }
            let axis = gizmo::rotate_vector(d.status);
            for (h, r) in d.held.iter().zip(d.turned.iter_mut()) {
                *r = gizmo::rotate_local(*r, axis, amount);
                items.push((h.id, TransformEdit::Rotation(*r)));
            }
            "Rotate"
        }
        GizmoType::None => return,
    };
    if items.is_empty() {
        return;
    }
    d.svc.store.post(
        SceneCmd::Transform { items, members: true, label: label.into(), merge: Some(d.merge.clone()) },
        editor.ui_origin(),
    );
}

/// Where a placed object lands (`PickGround`): with Object Snap on the surface of a scene
/// object or a spawn point (the snap list's objects only when it is used), otherwise the grid's
/// plane, on the grid's steps when the grid snap is on. The surface's normal comes with it.
pub(crate) async fn pick_ground(
    editor: &Arc<Editor>,
    svc: &Arc<SceneService>,
    ray: Ray,
    object_snap: bool,
    grid: Option<f32>,
    far: f32,
) -> Option<(Vec3, Vec3)> {
    if object_snap {
        let (snap, pick) = svc.pick().await;
        let lib = svc.library.clone();
        let hit = editor
            .exec
            .cpu(move || {
                let only: Option<Vec<ObjId>> = snap.tools.snap_use.then(|| snap.tools.snap.clone());
                pick.ray_pick(&snap, &lib, ray.origin, ray.dir, far, &|o: &Object| {
                    matches!(o.class, ObjClass::SceneObject | ObjClass::SpawnPoint)
                        && o.flags.visible
                        && only.as_ref().is_none_or(|l| l.contains(&o.id))
                })
            })
            .await;
        // oracle: the C++ vertex snap works out the nearest vertex and then lands on the hit
        // point anyway (`RetrieveSceneObjPointAndNormal`); so does this
        if let Some(h) = hit {
            return Some((h.point, h.normal.unwrap_or(Vec3::Y)));
        }
    }
    let t = ray.intersect_plane_y(0.0).filter(|t| *t > 0.0)?;
    let mut p = ray.at(t);
    if let Some(step) = grid {
        p.x = gizmo::snapto(p.x, step);
        p.z = gizmo::snapto(p.z, step);
        p.y = 0.0;
    }
    Some((p, Vec3::Y))
}

/// A click of Place (`DefaultAddObject` and the tools' `AddStart`): the current class's object
/// at the ground hit, stood on the surface with Align to Surface Normal; Alt keeps placing.
fn place(editor: &Arc<Editor>, vp: &Arc<Viewport>, ray: Ray, b: Buttons) {
    let Ok(svc) = SceneService::of(editor) else { return };
    let (object_snap, normal_align, grid, far) = vp.read(|s| {
        (
            s.settings.object_snap,
            s.settings.normal_align,
            s.snap_on(GizmoType::Move).then_some(s.step(GizmoType::Move)),
            s.nav.far,
        )
    });
    let class = svc.store.snapshot().tools.target;
    let item = vp.read(|s| s.place.get(&class).cloned());
    let ed = editor.clone();
    let vp = vp.clone();
    editor.exec.spawn(async move {
        let Some((at, normal)) = pick_ground(&ed, &svc, ray, object_snap, grid, far).await else { return };
        let origin = ed.ui_origin();
        let up = normal_align.then_some(normal);
        let snap = svc.store.snapshot();
        let placed: Option<ObjId> = match class {
            ObjClass::Group => {
                // `TUI_ControlGroupAdd`: the Group tool's current library group
                let Some(reference) = item else {
                    ed.log.info("Nothing selected.");
                    return;
                };
                let mut args = gz_editor::Args::new();
                args.insert("action".into(), Value::String("place".into()));
                args.insert("ref".into(), Value::String(reference.to_string()));
                for (k, v) in [("x", at.x), ("y", at.y), ("z", at.z)] {
                    args.insert(k.into(), json!(v));
                }
                let reply = ed.call("group_tool", args, origin).await;
                if reply.get("ok").and_then(Value::as_bool) != Some(true) {
                    ed.log.error(
                        reply.get("error").and_then(Value::as_str).unwrap_or("the group could not be placed").to_owned(),
                    );
                    return;
                }
                let name = reply.get("object").and_then(|o| o.get("name")).and_then(Value::as_str).unwrap_or("");
                svc.store.snapshot().find(ObjClass::Group, name).map(|o| o.id)
            }
            ObjClass::SpawnPoint => {
                let Some(section) = item else {
                    ed.log.info("Nothing selected.");
                    return;
                };
                let mut args = gz_editor::Args::new();
                args.insert("section".into(), Value::String(section.to_string()));
                for (k, v) in [("x", at.x), ("y", at.y), ("z", at.z)] {
                    args.insert(k.into(), json!(v));
                }
                let reply = ed.call("place_spawn", args, origin).await;
                if reply.get("ok").and_then(Value::as_bool) != Some(true) {
                    ed.log.error(
                        reply
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("the spawn point could not be placed")
                            .to_owned(),
                    );
                    return;
                }
                let name =
                    reply.get("object").and_then(|o| o.get("name")).and_then(Value::as_str).unwrap_or("");
                let id = svc.store.snapshot().find(ObjClass::SpawnPoint, name).map(|o| o.id);
                if let (Some(id), Some(_)) = (id, up) {
                    let _ = svc
                        .store
                        .send(
                            SceneCmd::Transform {
                                items: vec![(id, TransformEdit::MoveTo { position: at, up })],
                                members: true,
                                label: "Move".into(),
                                merge: None,
                            },
                            origin,
                        )
                        .await;
                }
                id
            }
            ObjClass::SceneObject | ObjClass::Particles => {
                // `TUI_ControlObjectAdd::Start`: Randomize Added Objects picks the object from its
                // list, and the scale and rotation from its ranges
                let random = random_append(&snap, class);
                let mut rng = gz_particles::Rng::new(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(1, |d| d.subsec_nanos() as i32),
                );
                let item = match &random {
                    Some(r) if !r.objects.is_empty() => {
                        let i = rng.rand_i_max(i32::try_from(r.objects.len()).unwrap_or(i32::MAX)) as usize;
                        r.objects.get(i).cloned().or(item)
                    }
                    _ => item,
                };
                let Some(reference) = item else {
                    ed.log.info("Nothing selected.");
                    return;
                };
                let data = if class == ObjClass::SceneObject {
                    let lib = svc.library.clone();
                    let key = gz_scene::library::normalize(&reference);
                    if ed.exec.io(move || lib.load(&key)).await.is_err() {
                        ed.log.error("TUI_ControlObjectAdd:: Can't load reference object.");
                        return;
                    }
                    ObjData::SceneObject(SceneObjectData::library(&reference))
                } else {
                    ObjData::Particles(ParticlesData {
                        reference: reference.clone(),
                        ..ParticlesData::default()
                    })
                };
                let mut o = Object::new(class, &snap.gen_name(Some(&reference)), data);
                if let Some(r) = &random {
                    if r.flags & gz_scene::tools::FL_APPEND_RANDOM_ROTATION != 0 {
                        o.xform.rotation = Vec3::new(
                            rng.rand_f_range(r.min_rotation.x, r.max_rotation.x),
                            rng.rand_f_range(r.min_rotation.y, r.max_rotation.y),
                            rng.rand_f_range(r.min_rotation.z, r.max_rotation.z),
                        );
                    }
                    if r.flags & gz_scene::tools::FL_APPEND_RANDOM_SCALE != 0 {
                        o.xform.scale = if r.flags & gz_scene::tools::FL_APPEND_RANDOM_PROPORTIONAL != 0 {
                            Vec3::splat(rng.rand_f_range(r.min_scale.x, r.max_scale.x))
                        } else {
                            Vec3::new(
                                rng.rand_f_range(r.min_scale.x, r.max_scale.x),
                                rng.rand_f_range(r.min_scale.y, r.max_scale.y),
                                rng.rand_f_range(r.min_scale.z, r.max_scale.z),
                            )
                        };
                    }
                }
                stand(&mut o, at, up);
                send_place(&svc, o, origin).await
            }
            ObjClass::Light
            | ObjClass::Glow
            | ObjClass::SoundSrc
            | ObjClass::SoundEnv
            | ObjClass::Shape
            | ObjClass::FogVolume
            | ObjClass::Way => {
                let data = match class {
                    ObjClass::Way => ObjData::Way(WayData::build(&[at], false)),
                    ObjClass::Shape => crate::objtools::new_shape(&ed),
                    _ => default_data(class),
                };
                let mut o = Object::new(class, &snap.gen_name(Some(class.name())), data);
                stand(&mut o, at, up);
                if let ObjData::SoundSrc(s) = &mut o.data {
                    s.position = at;
                }
                send_place(&svc, o, origin).await
            }
            _ => {
                ed.log.info(format!("The {} tool places nothing with a click.", tool_title(class)));
                return;
            }
        };
        if placed.is_some() && !b.alt {
            vp.with(|s| s.tool.action = Action::Select);
        }
        ed.exec.wake().ping();
    });
}

/// The Object tool's Append Random settings while Randomize Added Objects is on.
struct RandomAppend {
    flags: u32,
    objects: Vec<gz_base::SmolStr>,
    min_scale: Vec3,
    max_scale: Vec3,
    min_rotation: Vec3,
    max_rotation: Vec3,
}

fn random_append(snap: &gz_scene::SceneState, class: ObjClass) -> Option<RandomAppend> {
    if class != ObjClass::SceneObject {
        return None;
    }
    let part = snap.tools.part(ObjClass::SceneObject);
    if part.flags & gz_scene::tools::FL_APPEND_RANDOM == 0 {
        return None;
    }
    let gz_scene::tools::ToolOpts::Objects(o) = &part.opts else { return None };
    Some(RandomAppend {
        flags: part.flags,
        objects: o.objects.clone(),
        min_scale: o.min_scale,
        max_scale: o.max_scale,
        min_rotation: o.min_rotation,
        max_rotation: o.max_rotation,
    })
}

/// `MoveTo(p, n)`: the position, and with a normal the rotation that stands it on the surface.
fn stand(o: &mut Object, at: Vec3, up: Option<Vec3>) {
    if let Some(n) = up {
        let k = gz_scene::math::rotation_matrix(o.rotation()).row(2);
        o.xform.rotation = gz_scene::math::normal_align(n, k);
    }
    o.xform.position = at;
}

async fn send_place(svc: &SceneService, o: Object, origin: gz_editor::Origin) -> Option<ObjId> {
    match svc.store.send(SceneCmd::Place(Box::new(o)), origin).await {
        Ok(gz_scene::SceneReply::Id(id)) => Some(id),
        _ => None,
    }
}
