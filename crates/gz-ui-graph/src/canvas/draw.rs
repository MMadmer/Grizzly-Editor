//! What a frame of the canvas paints: the grid, the comment frames, the wires, the nodes at the
//! detail the zoom allows, what a gesture shows (a wire being drawn, alignment guides, the cut
//! line, an action being moved, the marquee), the minimap, the edge pills and the status.

use egui::epaint::Shadow;
use egui::text::TextWrapping;

use super::*;

impl Detail {
    /// Less detail is a lower rank.
    fn rank(self) -> u8 {
        match self {
            Detail::Block => 0,
            Detail::Header => 1,
            Detail::Full => 2,
        }
    }
}

/// The colour of a wire or a pin by what its pin means.
fn role_color(p: &Palette, role: PinRole, by_pin: bool) -> Color32 {
    if !by_pin {
        return p.wire_base;
    }
    match role {
        PinRole::Default => p.wire_base,
        PinRole::Failure => p.danger,
        // the other ways out by their place, through the families' hues
        PinRole::Other(k) => p.family(k % 7 + 2),
    }
}

fn pin_color(p: &Palette, role: PinRole, by_pin: bool) -> Color32 {
    if role == PinRole::Default || !by_pin { p.pin_exec } else { role_color(p, role, true) }
}

/// A screen polyline.
fn screen_points(c: &Canvas, view: &ViewMirror, pts: &[[f32; 2]]) -> Vec<Pos2> {
    pts.iter().map(|q| c.to_screen(view, *q)).collect()
}

/// A shadow scaled with the canvas.
fn scaled_shadow(sh: Shadow, s: f32, alpha: f32) -> Shadow {
    let k = s.clamp(0.0, 2.0);
    Shadow {
        offset: [(f32::from(sh.offset[0]) * k).round() as i8, (f32::from(sh.offset[1]) * k).round() as i8],
        blur: (f32::from(sh.blur) * k).round().clamp(0.0, 255.0) as u8,
        spread: 0,
        color: Palette::alpha(sh.color, alpha),
    }
}

impl Canvas {
    /// Draws the canvas into `rect` and handles its input.
    #[allow(clippy::too_many_lines)]
    pub fn draw(&mut self, ui: &mut Ui, rect: Rect, cx: &mut CanvasCx<'_>) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let ppp = ctx.pixels_per_point();
        self.origin = rect.min;
        self.size = vec2(rect.width().max(32.0), rect.height().max(32.0));
        self.drawn_frame = ctx.cumulative_frame_nr();
        cx.view.center = [layout::sane(cx.view.center[0]), layout::sane(cx.view.center[1])];
        if let Some(st) = cx.doc_status.take() {
            self.set_status(&ctx, &st);
        }
        // a commit in flight is drawn until the scene that holds it arrives
        if self.settle.as_ref().is_some_and(|s| cx.scene.is_some_and(|sc| sc.edit != s.from_edit)) {
            self.settle = None;
        }

        // deferred framing needs the rect and the scene
        if let Some(sc) = cx.scene {
            if self.want_frame_all {
                self.want_frame_all = false;
                self.frame_all_now(sc, cx.view);
            }
            if self.want_frame_sel {
                self.want_frame_sel = false;
                self.frame_selection_now(sc, cx.view);
            }
            // framing picks a zoom of its own; an explicit one was asked for later
            if let Some(z) = self.want_zoom.take() {
                cx.view.zoom = z;
            }
        }

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, p.canvas);
        let resp = ui.interact(rect, self.id, Sense::click_and_drag());
        let covered =
            ctx.input(|i| i.pointer.hover_pos()).is_some_and(|m| cx.covered.iter().any(|r| r.contains(m)));
        self.hovered = (resp.hovered() && !covered) || resp.dragged();
        let menu_up = egui::Popup::is_any_open(&ctx) || self.frame_title.is_some();
        if resp.clicked() || resp.drag_started() || resp.secondary_clicked() {
            self.focused = true;
        } else if ctx.input(|i| i.pointer.any_pressed()) && !self.hovered && !menu_up {
            self.focused = false;
        }

        let Some(sc) = cx.scene else {
            gz_ui::state::busy_line(
                &mut ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(16.0))),
                "Reading the graph",
            );
            return;
        };
        let mouse = ctx.input(|i| i.pointer.hover_pos()).unwrap_or(Pos2::new(-1.0e6, -1.0e6));
        self.last_mouse = mouse;
        let grab = Self::pin_grab(cx.view, ppp);
        self.update_hot(sc, cx, mouse, grab, menu_up, ppp);
        self.tip.clear();

        self.draw_grid(&painter, &p, cx.view);
        let view_r = self.viewport(cx.view).unwrap_or([0.0; 4]);
        self.draw_frames(&painter, &p, sc, cx, mouse, ppp);
        let pad = 64.0 / Self::scale(cx.view);
        let links =
            self.visible_links(sc, [view_r[0] - pad, view_r[1] - pad, view_r[2] + pad, view_r[3] + pad]);
        let now = ctx.input(|i| i.time);
        self.draw_links(&painter, &p, sc, cx, &links, ppp, now);
        self.draw_linking(&painter, &p, sc, cx.view, mouse);

        self.scratch.clear();
        let mut scratch = std::mem::take(&mut self.scratch);
        sc.nodes_in(view_r, &mut scratch);
        let mut visible = self.dedupe(sc.nodes.len(), &scratch);
        self.scratch = scratch;
        if self.moving() {
            if let Some(d) = &self.drag {
                visible.extend(d.nodes.iter().map(|(i, _, _)| *i));
            }
            if let Some(f) = &self.frame_drag {
                visible.extend(f.inside.iter().copied());
            }
            visible.sort_unstable();
            visible.dedup();
        }
        let mut least: Option<Detail> = None;
        for &i in &visible {
            let Some(n) = sc.nodes.get(i) else { continue };
            let d = Self::detail(cx.view, ppp, n.size[0]);
            least = Some(least.map_or(d, |l| if d.rank() < l.rank() { d } else { l }));
            self.draw_node(&painter, &p, sc, cx, i, mouse, ppp);
        }
        self.drawn_detail = least.unwrap_or_else(|| Self::detail(cx.view, ppp, layout::NODE_MIN_W));
        self.drawn_title_px = Self::title_px(cx.view, ppp);
        self.draw_guides(&painter, &p, cx.view);
        self.draw_action_drag(&painter, &p, sc, cx, mouse, ppp);
        self.draw_marquee(&painter, &p, mouse);
        self.draw_cut(&painter, &p, mouse);
        self.draw_minimap(&painter, &p, sc, cx, mouse);
        self.draw_beyond(&painter, &p, sc, cx.view, mouse);
        self.draw_status(&painter, &p, &ctx);
        if sc.nodes.is_empty() {
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                "Right-click or press Tab to add a node",
                Font::Body.id(),
                p.text_muted,
            );
        }

        self.handle_input(&ctx, sc, cx, mouse, grab, &resp, menu_up, ppp);
        self.draw_frame_title_edit(ui, sc, cx);
        self.draw_menus(ui, sc, cx);
        if self.hovered && !self.tip.is_empty() && !menu_up {
            resp.on_hover_text_at_pointer(self.tip.clone());
        }
    }

    fn update_hot(&mut self, sc: &Scene, cx: &CanvasCx<'_>, mouse: Pos2, grab: f32, menu_up: bool, ppp: f32) {
        self.hot_node = None;
        self.hot_pin = None;
        self.hot_link = None;
        self.hot_action = None;
        self.hot_frame = None;
        self.hot_knot = None;
        if !self.hovered || menu_up {
            return;
        }
        let view = &*cx.view;
        match (&self.linking, &self.rewire) {
            (Some(Linking::From { node, .. }), _) | (None, Some(Rewire { from: node, target: true, .. })) => {
                let from = sc.node_of(node);
                let hit = self
                    .hit_node(sc, view, mouse)
                    .or_else(|| self.hit_node_near(sc, view, mouse, from, grab));
                if let Some(n) = hit
                    && Some(n) != from
                    && sc.nodes.get(n).is_some_and(|x| !x.trigger)
                {
                    self.hot_node = Some(n);
                }
                return;
            }
            (Some(Linking::Into { .. }), _) | (None, Some(Rewire { target: false, .. })) => {
                if let Some((n, k)) = self.hit_any_pin(sc, view, mouse, grab) {
                    self.hot_node = Some(n);
                    self.hot_pin = Some(k);
                } else if let Some(n) = self.hit_node(sc, view, mouse) {
                    self.hot_node = Some(n);
                }
                return;
            }
            _ => {}
        }
        if self.drag.as_ref().is_some_and(|d| d.moved)
            || self.marquee.is_some()
            || self.cut.is_some()
            || self.pan.is_some_and(|(_, m)| m)
            || self.act_drag.as_ref().is_some_and(|a| a.dragging)
            || self.frame_drag.is_some()
            || self.knot_drag.as_ref().is_some_and(|k| k.moved)
        {
            return;
        }
        // pins are drawn (and taken) once a node is more than a block
        let mut best = f32::MAX;
        for i in self.candidates(sc, view, mouse, grab) {
            let Some(n) = sc.nodes.get(i) else { continue };
            if Self::detail(view, ppp, n.size[0]) == Detail::Block {
                continue;
            }
            if let Some((k, d2)) = self.hit_pin(sc, view, i, mouse, grab)
                && d2 < best
            {
                best = d2;
                self.hot_node = Some(i);
                self.hot_pin = Some(k);
            }
            if !n.trigger
                && let Some(d2) = Self::near(mouse, self.input_pin(sc, view, i), grab)
                && d2 < best
            {
                best = d2;
                self.hot_node = Some(i);
                self.hot_pin = None;
            }
        }
        let over = self.hit_node(sc, view, mouse);
        if self.hot_node.is_none()
            && let Some(o) = over
        {
            let d = sc.nodes.get(o).map_or(Detail::Block, |n| Self::detail(view, ppp, n.size[0]));
            self.hot_action = self.hit_action(sc, view, o, mouse, d);
        }
        // a pin wins over a wire it starts from, and a node body swallows the wires behind it
        if self.hot_node.is_none() && over.is_none() {
            self.hot_knot = self.hit_knot(sc, view, mouse, (5.0 * Self::scale(view)).max(6.0));
            if self.hot_knot.is_none() {
                self.hot_link = self.hit_link(sc, view, mouse);
            }
            if self.hot_link.is_none() && self.hot_knot.is_none() {
                self.hot_frame = self.hit_frame(sc, view, mouse);
            }
        }
    }

    fn draw_grid(&self, painter: &Painter, p: &Palette, view: &ViewMirror) {
        let s = Self::scale(view);
        let mut step = GRID * 4.0 * s;
        while step < 24.0 {
            step *= 2.0;
        }
        let o = self.to_screen(view, [0.0, 0.0]);
        let sx = o.x - ((o.x - self.origin.x) / step).floor() * step;
        let sy = o.y - ((o.y - self.origin.y) / step).floor() * step;
        if !sx.is_finite() || !sy.is_finite() {
            return;
        }
        let nx = (self.size.x / step) as i32 + 2;
        let ny = (self.size.y / step) as i32 + 2;
        if !(1..=4096).contains(&nx) || !(1..=4096).contains(&ny) {
            return;
        }
        let stroke = Stroke::new(1.0, p.grid_line);
        let (x1, y1) = (self.origin.x + self.size.x, self.origin.y + self.size.y);
        for i in 0..nx {
            let x = sx + i as f32 * step;
            if x >= x1 {
                break;
            }
            if x >= self.origin.x {
                painter.line_segment([pos2(x, self.origin.y), pos2(x, y1)], stroke);
            }
        }
        for i in 0..ny {
            let y = sy + i as f32 * step;
            if y >= y1 {
                break;
            }
            if y >= self.origin.y {
                painter.line_segment([pos2(self.origin.x, y), pos2(x1, y)], stroke);
            }
        }
    }

    /// The comment frames, under everything else: a tinted area, a title strip, a resize handle.
    fn draw_frames(
        &mut self,
        painter: &Painter,
        p: &Palette,
        sc: &Scene,
        cx: &CanvasCx<'_>,
        mouse: Pos2,
        ppp: f32,
    ) {
        let view = &*cx.view;
        let s = Self::scale(view);
        let canvas = Rect::from_min_size(self.origin, self.size);
        for f in &sc.frames {
            let r = self.frame_rect(f);
            let sr =
                Rect::from_min_max(self.to_screen(view, [r[0], r[1]]), self.to_screen(view, [r[2], r[3]]));
            if !sr.intersects(canvas) {
                continue;
            }
            let tone = p.family(usize::from(f.tone));
            let rr = radius(8.0 * s);
            painter.rect_filled(sr, rr, p.frame_fill);
            painter.rect_filled(sr, rr, Palette::alpha(tone, 0.08));
            let strip_h = (FRAME_TITLE * s).min(sr.height());
            let strip = Rect::from_min_size(sr.min, vec2(sr.width(), strip_h));
            painter.rect_filled(
                strip,
                CornerRadius { nw: rr.nw, ne: rr.ne, sw: 0, se: 0 },
                Palette::alpha(tone, 0.24),
            );
            let hot = self.hot_frame.is_some_and(|(i, _)| i == f.index)
                || self.frame_drag.as_ref().is_some_and(|d| d.index == f.index);
            painter.rect_stroke(
                sr,
                rr,
                Stroke::new(if hot { 1.5 } else { 1.0 }, Palette::alpha(tone, if hot { 0.9 } else { 0.55 })),
                StrokeKind::Inside,
            );
            // the title stays readable however far out the view is
            let line_px = (TITLE_LINE * 1.1 * s * ppp).clamp(12.0, 40.0);
            let font = Self::font_px(Font::Strong, line_px, ppp);
            if self.frame_title.as_ref().is_none_or(|(i, _, _)| *i != f.index) {
                paint_fit(
                    painter,
                    &font,
                    pos2(strip.min.x + (10.0 * s).max(6.0), strip.min.y + strip_h.max(line_px / ppp) * 0.5),
                    p.text,
                    &f.title,
                    (sr.width() - 20.0 * s).max(0.0),
                );
            }
            // the resize handle in the bottom right corner
            let h = (FRAME_HANDLE * s).max(6.0);
            let c = sr.max;
            for k in 1..=2 {
                let d = h * k as f32 / 2.0;
                painter.line_segment(
                    [pos2(c.x - d, c.y - 2.0), pos2(c.x - 2.0, c.y - d)],
                    Stroke::new(1.0, Palette::alpha(tone, 0.8)),
                );
            }
            if self.hovered && self.hot_frame.is_some_and(|(i, _)| i == f.index) && self.tip.is_empty() {
                self.tip = if self.hot_frame.is_some_and(|(_, resize)| resize) {
                    "Drag to resize the frame".to_owned()
                } else {
                    format!(
                        "{}\nDrag to move the frame and what it holds, double-click to rename it",
                        f.title
                    )
                };
            }
            let _ = mouse;
        }
    }

    /// The nodes downstream of the selection, as a flag per wire (Flow Bubbles).
    fn flowing(&mut self, sc: &Scene, view: &ViewMirror) -> Vec<bool> {
        if let Some((edit, sel, flags)) = &self.flowing
            && *edit == sc.edit
            && *sel == view.selection
        {
            return flags.clone();
        }
        let mut reached = vec![false; sc.nodes.len()];
        let mut stack: Vec<usize> = view.selection.iter().filter_map(|id| sc.node_of(id)).collect();
        for &i in &stack {
            if let Some(r) = reached.get_mut(i) {
                *r = true;
            }
        }
        let mut out_of: Vec<Vec<usize>> = vec![Vec::new(); sc.nodes.len()];
        for (li, l) in sc.links.iter().enumerate() {
            if let Some(v) = out_of.get_mut(l.from as usize) {
                v.push(li);
            }
        }
        let mut flags = vec![false; sc.links.len()];
        while let Some(i) = stack.pop() {
            for &li in out_of.get(i).map_or(&[][..], Vec::as_slice) {
                if let Some(f) = flags.get_mut(li) {
                    *f = true;
                }
                let to = sc.links.get(li).map_or(usize::MAX, |l| l.to as usize);
                if let Some(r) = reached.get_mut(to)
                    && !*r
                {
                    *r = true;
                    stack.push(to);
                }
            }
        }
        self.flowing = Some((sc.edit, view.selection.clone(), flags.clone()));
        flags
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn draw_links(
        &mut self,
        painter: &Painter,
        p: &Palette,
        sc: &Scene,
        cx: &CanvasCx<'_>,
        links: &[usize],
        ppp: f32,
        now: f64,
    ) {
        let view = &*cx.view;
        let s = Self::scale(view);
        let set = sc.wires;
        let w = (set.thickness * s).max(1.0 / ppp);
        let any_selected = !view.selection.is_empty();
        let flowing = if set.bubbles && any_selected { Some(self.flowing(sc, view)) } else { None };
        let on_wire = self.drag.as_ref().and_then(|d| d.on_wire);
        let pin_r = (2.0 / ppp).max(PIN_R * s);
        let mut bubbles = false;
        for &i in links {
            let Some(l) = sc.links.get(i) else { continue };
            let (Some(from), Some(to)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) else {
                continue;
            };
            let touches = view.is_selected(&from.id) || view.is_selected(&to.id);
            let hot = self.hot_link == Some(i) || on_wire == Some(i);
            let base = role_color(p, l.role, set.by_pin);
            let mut col = if hot {
                p.wire_hot
            } else if touches {
                p.wire_selected
            } else {
                base
            };
            if set.dim && any_selected && !touches && !hot {
                col = Palette::alpha(col, 0.15);
            }
            if from.disabled || to.disabled {
                col = Palette::alpha(col, 0.6);
            }
            let pts = screen_points(self, view, &self.link_points(sc, i));
            if pts.len() < 2 {
                continue;
            }
            if hot {
                painter.add(Shape::line(
                    pts.clone(),
                    Stroke::new(w * 3.0 + 2.0, Palette::alpha(p.wire_hot, 0.25)),
                ));
            }
            let dashed = l.back || from.disabled || to.disabled;
            if dashed {
                let dash = (8.0 * s).max(4.0);
                painter.extend(Shape::dashed_line(&pts, Stroke::new(w, col), dash, dash * 0.6));
            } else {
                painter.add(Shape::Path(PathShape::line(pts.clone(), PathStroke::new(w, col))));
            }
            // the arrowhead ends at the input pin's rim, never on the dot
            if set.arrows
                && let (Some(&end), Some(&prev)) = (pts.last(), pts.get(pts.len() - 2))
            {
                let d = end - prev;
                if d.length() > 0.001 {
                    let dir = d.normalized();
                    let tip = end - dir * pin_r;
                    let a = (7.0 * s).max(4.0) + w;
                    let b = tip - dir * a;
                    let half = vec2(-dir.y, dir.x) * a * 0.5;
                    painter.add(Shape::convex_polygon(vec![tip, b + half, b - half], col, Stroke::NONE));
                }
            }
            // the reroutes of the wire
            if !l.via.is_empty() {
                for (k, v) in l.via.iter().enumerate() {
                    let at = match self.knot_drag.as_ref().filter(|d| d.link == i && d.index == k && d.moved)
                    {
                        Some(d) => d.at,
                        None => *v,
                    };
                    let c = self.to_screen(view, at);
                    let hot_knot = self.hot_knot == Some((i, k));
                    let r = (4.5 * s).max(3.0) * if hot_knot { 1.3 } else { 1.0 };
                    painter.circle_filled(c, r, if hot_knot { p.wire_hot } else { col });
                    painter.circle_stroke(c, r, Stroke::new(1.0, p.canvas));
                }
            }
            if let Some(flags) = &flowing
                && flags.get(i).copied().unwrap_or(false)
            {
                let world = self.link_points(sc, i);
                let len = wires::length(&world);
                if len > 1.0 {
                    bubbles = true;
                    let spacing = 64.0;
                    let offset = ((now as f32) * BUBBLE_SPEED) % spacing;
                    let mut d = offset;
                    while d < len {
                        if let Some((q, _)) = wires::point_at(&world, d / len) {
                            painter.circle_filled(self.to_screen(view, q), (3.0 * s).max(2.0), p.wire_hot);
                        }
                        d += spacing;
                    }
                }
            }
        }
        // the bubbles move only while they are on and on screen
        if bubbles {
            painter.ctx().request_repaint();
        }
    }

    /// A wire being drawn or moved, in the graph's own style.
    fn draw_linking(&self, painter: &Painter, p: &Palette, sc: &Scene, view: &ViewMirror, mouse: Pos2) {
        let m = self.to_world(view, mouse);
        let (a, b) = match (&self.linking, &self.rewire) {
            (Some(Linking::From { node, pin }), _) => {
                let Some(i) = sc.node_of(node) else { return };
                let Some(k) = sc.nodes.get(i).and_then(|n| n.pin_index(pin)) else { return };
                (self.output_pin_w(sc, i, k), m)
            }
            (Some(Linking::Into { node }), _) => {
                let Some(i) = sc.node_of(node) else { return };
                (m, self.input_pin_w(sc, i))
            }
            (None, Some(r)) if r.target => {
                let Some(i) = sc.node_of(&r.from) else { return };
                let Some(k) = sc.nodes.get(i).and_then(|n| n.pin_index(&r.pin)) else { return };
                (self.output_pin_w(sc, i, k), m)
            }
            (None, Some(r)) => {
                let Some(i) = sc.node_of(&r.to) else { return };
                (m, self.input_pin_w(sc, i))
            }
            _ => return,
        };
        let wcx = wires::WireCx { flow: sc.flow, style: sc.wires.style, diagonal: sc.wires.diagonal };
        let w = wires::build(&wcx, a, b, &[], wires::Lanes::default(), &[], &mut Vec::new());
        let pts = screen_points(self, view, &w.points);
        painter.add(Shape::line(pts, Stroke::new(2.0, p.wire_hot)));
    }

    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn draw_node(
        &mut self,
        painter: &Painter,
        p: &Palette,
        sc: &Scene,
        cx: &CanvasCx<'_>,
        i: usize,
        mouse: Pos2,
        ppp: f32,
    ) {
        let Some(n) = sc.nodes.get(i) else { return };
        let view = &*cx.view;
        let s = Self::scale(view);
        let r = self.rect_of(sc, view, i);
        let (a, b) = (r.min, r.max);
        let detail = Self::detail(view, ppp, n.size[0]);
        let selected = view.is_selected(&n.id);
        // an unreachable node is dimmed, never hidden: it is still the author's to wire; a disabled
        // one is drawn at 40 %
        let mut alpha = if n.reachable { 1.0 } else { 0.55 };
        if n.disabled {
            alpha *= 0.4;
        }
        let fam = |f: Family, k: f32| look::family_color(p, f, k);
        let marks = cx.data.marks(&n.id);
        let hovered_node = self.hovered && r.contains(mouse) && !self.minimap_hovered;
        let rnd = (if n.trigger { HEADER * 0.5 } else { 6.0 }) * s;
        let mark_color = marks.map(|m| if m.severity >= 2 { p.error } else { p.warning });

        if detail == Detail::Block {
            painter.rect_filled(r, radius(rnd.min(r.height() * 0.5)), fam(n.family, 0.85 * alpha));
            if let Some(c) = mark_color {
                painter.rect_filled(
                    Rect::from_min_size(a, vec2(r.width(), (r.height() * 0.25).max(1.0))),
                    0.0,
                    c,
                );
            }
            if selected {
                painter.rect_stroke(
                    r.expand(1.0),
                    radius(rnd),
                    Stroke::new(2.0, p.accent_bright),
                    StrokeKind::Outside,
                );
            }
            if hovered_node && self.tip.is_empty() {
                self.tip = node_tip(n, marks);
            }
            return;
        }

        painter.add(scaled_shadow(p.shadow_card, s, alpha).as_shape(r, radius(rnd)));
        painter.rect_filled(r, radius(rnd), Palette::alpha(p.raised, alpha.max(0.4)));
        if detail == Detail::Header {
            self.draw_header_only(painter, p, n, r, rnd, alpha, mark_color, ppp);
        } else {
            let header = (HEADER * s).min(r.height());
            let collapsed_box = n.b.h <= HEADER + 0.5;
            let hb = pos2(b.x, a.y + header);
            let head_corners = if collapsed_box {
                radius(rnd)
            } else {
                CornerRadius { nw: radius(rnd).nw, ne: radius(rnd).ne, sw: 0, se: 0 }
            };
            let head = Rect::from_min_max(a, hb);
            painter.rect_filled(head, head_corners, Palette::alpha(p.hover, alpha.max(0.4)));
            painter.rect_filled(head, head_corners, fam(n.family, 0.22 * alpha.max(0.4)));
            if n.disabled {
                hatch(painter, head, p.text_muted, s);
            }
            if !collapsed_box {
                painter.line_segment(
                    [pos2(a.x + rnd * 0.5, hb.y), pos2(b.x - rnd * 0.5, hb.y)],
                    Stroke::new((2.0 * s).max(1.0), fam(n.family, alpha)),
                );
            }
            // a node with problems says so on its top edge
            if let Some(c) = mark_color {
                let h = (3.0 * s).max(2.0);
                painter.rect_filled(
                    Rect::from_min_size(a, vec2(r.width(), h)),
                    CornerRadius { nw: radius(rnd).nw, ne: radius(rnd).ne, sw: 0, se: 0 },
                    c,
                );
            }
            self.draw_header(painter, p, cx, n, a, b, alpha, mouse, ppp);
            if !n.collapsed {
                self.draw_body(painter, p, n, a, b, alpha, view, ppp);
                if n.b.actions > 0 {
                    self.draw_actions(painter, p, n, cx, i, a, alpha, ppp);
                }
            }
        }

        let border = if selected {
            Stroke::new(2.0, p.accent_bright)
        } else if hovered_node {
            Stroke::new(1.0, Palette::alpha(p.border_strong, alpha.max(0.4)))
        } else {
            Stroke::new(1.0, Palette::alpha(p.border, alpha.max(0.4)))
        };
        painter.rect_stroke(r, radius(rnd), border, StrokeKind::Inside);
        self.draw_pins(painter, p, sc, cx, i, detail, alpha, mouse, ppp);
        if detail == Detail::Header && hovered_node && self.tip.is_empty() && self.hot_node.is_none() {
            self.tip = node_tip(n, marks);
        }
    }

    /// A node too small for its body: its title at a fixed size over the whole box.
    #[allow(clippy::too_many_arguments)]
    fn draw_header_only(
        &self,
        painter: &Painter,
        p: &Palette,
        n: &SceneNode,
        r: Rect,
        rnd: f32,
        alpha: f32,
        mark: Option<Color32>,
        ppp: f32,
    ) {
        let fam = |k: f32| look::family_color(p, n.family, k);
        painter.rect_filled(r, radius(rnd), fam(0.30 * alpha.max(0.4)));
        if n.disabled {
            hatch(painter, r, p.text_muted, Self::scale_of(r, n));
        }
        if let Some(c) = mark {
            painter.rect_filled(Rect::from_min_size(r.min, vec2(r.width(), 2.0)), 0.0, c);
        }
        let line = FIXED_TITLE_PX / ppp;
        let font = Self::font_px(Font::Strong, FIXED_TITLE_PX, ppp);
        let inner = (r.width() - 8.0).max(line);
        let rows = ((r.height() - 4.0) / line).floor().max(1.0) as usize;
        let mut job = LayoutJob::single_section(
            n.title.clone(),
            TextFormat {
                font_id: font,
                color: Palette::alpha(p.text, alpha.max(0.5)),
                ..TextFormat::default()
            },
        );
        job.wrap = TextWrapping {
            max_width: inner,
            max_rows: rows.min(3),
            break_anywhere: false,
            overflow_character: Some('…'),
        };
        job.halign = egui::Align::Center;
        let galley = painter.ctx().fonts_mut(|f| f.layout_job(job));
        let size = galley.size();
        let at = pos2(r.center().x, r.center().y - size.y * 0.5);
        // a title taller than its box gets a plate of its own
        if size.y > r.height() {
            let plate = Rect::from_center_size(r.center(), vec2(size.x + 6.0, size.y + 2.0));
            painter.rect_filled(plate, CornerRadius::same(3), Palette::alpha(p.overlay, 0.92));
        }
        painter.galley(at, galley, p.text);
    }

    /// The scale a node is drawn at, from its screen box.
    fn scale_of(r: Rect, n: &SceneNode) -> f32 {
        if n.size[0] > 0.0 { r.width() / n.size[0] } else { 1.0 }
    }

    /// The header row at full detail: icon, title, muted id, badges.
    #[allow(clippy::too_many_arguments)]
    fn draw_header(
        &mut self,
        painter: &Painter,
        p: &Palette,
        cx: &CanvasCx<'_>,
        n: &SceneNode,
        a: Pos2,
        b: Pos2,
        alpha: f32,
        mouse: Pos2,
        ppp: f32,
    ) {
        let view = &*cx.view;
        let s = Self::scale(view);
        let header = HEADER * s;
        let inset = INSET * s;
        let strong = Self::font(Font::Strong, TITLE_LINE, view, ppp);
        let small = Self::font(Font::Body, BODY_LINE, view, ppp);
        let cy = a.y + header * 0.5;
        let mut x = a.x + inset;
        let mut right = b.x - inset;
        let icon = layout::ICON * s;
        icons::draw(
            painter,
            n.icon,
            Rect::from_min_size(pos2(x, cy - icon * 0.5), Vec2::splat(icon)),
            look::family_color(p, n.family, alpha.max(0.5)),
        );
        x += icon + 6.0 * s;
        let text_a = alpha.max(0.5);
        // badges from the right: problems, the bookmark, "once", "off"
        let badge = 14.0 * s;
        if let Some(m) = cx.data.marks(&n.id) {
            let c = if m.severity >= 2 { p.error } else { p.warning };
            let count = m.count.to_string();
            let tw = gz_ui::text::width(painter.ctx(), &small, &count);
            let pill = Rect::from_min_max(
                pos2(right - badge - tw - 8.0 * s, cy - badge * 0.55),
                pos2(right, cy + badge * 0.55),
            );
            painter.rect_filled(pill, radius(pill.height() * 0.5), Palette::alpha(c, 0.22));
            icons::draw(
                painter,
                if m.severity >= 2 { Icon::Error } else { Icon::Warning },
                Rect::from_min_size(pos2(pill.min.x + 3.0 * s, cy - badge * 0.4), Vec2::splat(badge * 0.8)),
                c,
            );
            painter.text(pos2(pill.max.x - 4.0 * s, cy), Align2::RIGHT_CENTER, count, small.clone(), c);
            if self.hovered && pill.contains(mouse) {
                self.tip = format!(
                    "{}\n{}",
                    m.first,
                    if m.count > 1 {
                        format!("and {} more in the Problems drawer", m.count - 1)
                    } else {
                        String::new()
                    }
                )
                .trim_end()
                .to_owned();
            }
            right = pill.min.x - 4.0 * s;
        }
        if cx.graph.canvas.bookmarks.iter().any(|bm| bm.node == n.id) {
            let bm = Rect::from_min_size(pos2(right - badge, cy - badge * 0.5), Vec2::splat(badge));
            icons::draw(painter, Icon::BookmarkFilled, bm, Palette::alpha(p.accent_bright, text_a));
            if self.hovered && bm.contains(mouse) {
                let names: Vec<&str> = cx
                    .graph
                    .canvas
                    .bookmarks
                    .iter()
                    .filter(|x| x.node == n.id)
                    .map(|x| x.name.as_str())
                    .collect();
                self.tip = format!("Bookmark: {}", names.join(", "));
            }
            right -= badge + 4.0 * s;
        }
        for (on, word) in [(n.disabled, "off"), (n.once, "once")] {
            if !on {
                continue;
            }
            let tw = gz_ui::text::width(painter.ctx(), &small, word);
            painter.text(
                pos2(right - tw, cy),
                Align2::LEFT_CENTER,
                word,
                small.clone(),
                Palette::alpha(p.text_muted, text_a),
            );
            right -= tw + 6.0 * s;
        }
        // the id gets what the title leaves, and never more than 40% of the row
        let room = (right - x).max(0.0);
        let title_w = gz_ui::text::width(painter.ctx(), &strong, &n.title);
        let id_w =
            gz_ui::text::width(painter.ctx(), &small, &n.id).min((room - title_w - 8.0 * s).max(room * 0.4));
        let line_px = small.size / 0.75;
        let show_id = id_w > line_px * 1.5;
        if show_id {
            paint_fit(
                painter,
                &small,
                pos2(right - id_w, cy),
                Palette::alpha(p.text_muted, text_a),
                &n.id,
                id_w,
            );
        }
        paint_fit(
            painter,
            &strong,
            pos2(x, cy),
            Palette::alpha(p.text, text_a),
            &n.title,
            if show_id { room - id_w - 8.0 * s } else { room },
        );
        let head = Rect::from_min_max(a, pos2(b.x, a.y + header));
        if self.hovered
            && head.contains(mouse)
            && self.tip.is_empty()
            && self.hot_node.is_none()
            && !n.desc.is_empty()
        {
            self.tip = format!("{}\n{}", n.title, n.desc);
        }
    }

    /// The comment, the lines.
    #[allow(clippy::too_many_arguments)]
    fn draw_body(
        &self,
        painter: &Painter,
        p: &Palette,
        n: &SceneNode,
        a: Pos2,
        b: Pos2,
        alpha: f32,
        view: &ViewMirror,
        ppp: f32,
    ) {
        let s = Self::scale(view);
        let body = Self::font(Font::Body, BODY_LINE, view, ppp);
        let inset = INSET * s;
        let inner = n.size[0] * s - inset * 2.0;
        let text_a = alpha.max(0.5);
        if let Some(cy) = n.b.comment_y
            && !n.comment.is_empty()
        {
            let y = a.y + (cy + COMMENT * 0.5) * s;
            let fitted = gz_ui::text::fit(painter.ctx(), &body, &n.comment, inner, gz_ui::Clip::End);
            let job = LayoutJob::single_section(
                fitted.text.to_string(),
                TextFormat {
                    font_id: body.clone(),
                    color: Palette::alpha(p.text_secondary, text_a),
                    italics: true,
                    ..TextFormat::default()
                },
            );
            let galley = painter.ctx().fonts_mut(|f| f.layout_job(job));
            let h = galley.size().y;
            painter.galley(pos2(a.x + inset, y - h * 0.5), galley, p.text_secondary);
        }
        for (k, line) in n.lines.iter().take(n.b.lines).enumerate() {
            let ly = a.y + (n.b.lines_y + LINE * k as f32 + LINE * 0.5) * s;
            if line.key.is_empty() {
                paint_fit(
                    painter,
                    &body,
                    pos2(a.x + inset, ly),
                    Palette::alpha(p.text_secondary, text_a),
                    &line.value,
                    inner,
                );
                continue;
            }
            // a parameter reads "name: value", a condition "Title what"
            let key = if line.cond.is_empty() { format!("{}:", line.key) } else { line.key.clone() };
            let share = if line.value.is_empty() {
                1.0
            } else if line.cond.is_empty() {
                0.45
            } else {
                0.6
            };
            let kw = paint_fit(
                painter,
                &body,
                pos2(a.x + inset, ly),
                Palette::alpha(p.text_muted, text_a),
                &key,
                inner * share,
            );
            let vx = a.x + inset + kw + 6.0 * s;
            paint_fit(
                painter,
                &body,
                pos2(vx, ly),
                Palette::alpha(p.text, text_a),
                &line.value,
                b.x - inset - vx,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_actions(
        &self,
        painter: &Painter,
        p: &Palette,
        n: &SceneNode,
        cx: &CanvasCx<'_>,
        index: usize,
        a: Pos2,
        alpha: f32,
        ppp: f32,
    ) {
        let view = &*cx.view;
        let s = Self::scale(view);
        let body = Self::font(Font::Body, BODY_LINE, view, ppp);
        let w = n.size[0] * s;
        let row_h = ACTION_ROW * s;
        let y0 = a.y + n.b.actions_y * s;
        let inset = INSET * s;
        let one_selected = view.is_selected(&n.id) && view.selection.len() == 1;
        let text_a = alpha.max(0.5);
        if n.b.lines > 0 || n.b.comment_y.is_some() {
            painter.line_segment(
                [pos2(a.x + inset, y0 - PAD * 0.5 * s), pos2(a.x + w - inset, y0 - PAD * 0.5 * s)],
                Stroke::new(1.0, Palette::alpha(p.border_subtle, text_a)),
            );
        }
        for (r, act) in n.actions.iter().enumerate() {
            let row = Rect::from_min_max(
                pos2(a.x + 3.0 * s, y0 + row_h * r as f32),
                pos2(a.x + w - 3.0 * s, y0 + row_h * (r + 1) as f32),
            );
            let selected = one_selected && view.slot == format!("{}:{}", slot_name(act.exit), act.index);
            let hot = self.hot_action.is_some_and(|h| h.node == index && h.row == r && !h.more);
            let dragged = self
                .act_drag
                .as_ref()
                .is_some_and(|d| d.dragging && d.node == n.id && d.exit == act.exit && d.index == act.index);
            if selected {
                painter.rect_filled(row, radius(3.0 * s), Palette::alpha(p.selection_focused, text_a));
            } else if hot {
                painter.rect_filled(row, radius(3.0 * s), Palette::alpha(p.hover, text_a));
            }
            if dragged {
                painter.rect_stroke(
                    row,
                    radius(3.0 * s),
                    Stroke::new((1.5 * s).max(1.0), p.accent_bright),
                    StrokeKind::Inside,
                );
            }
            let mut x = a.x + inset;
            let icon = layout::ROW_ICON * s;
            let cy = row.center().y;
            icons::draw(
                painter,
                act.icon,
                Rect::from_min_size(pos2(x, cy - icon * 0.5), Vec2::splat(icon)),
                look::family_color(p, act.family, text_a),
            );
            x += icon + 6.0 * s;
            let mut right = a.x + w - inset;
            if act.exit {
                let tw = gz_ui::text::width(painter.ctx(), &body, "exit");
                painter.text(
                    pos2(right - tw, cy),
                    Align2::LEFT_CENTER,
                    "exit",
                    body.clone(),
                    Palette::alpha(p.text_muted, text_a),
                );
                right -= tw + 6.0 * s;
            }
            let label = if act.summary.is_empty() { act.title.clone() } else { format!("{}:", act.title) };
            let tw =
                paint_fit(painter, &body, pos2(x, cy), Palette::alpha(p.text, text_a), &label, right - x);
            let sx = x + tw + 5.0 * s;
            if right - sx > body.size / 0.75 * 2.0 {
                paint_fit(
                    painter,
                    &body,
                    pos2(sx, cy),
                    Palette::alpha(p.text_secondary, text_a),
                    &act.summary,
                    right - sx,
                );
            }
        }
        if n.b.more {
            let top = y0 + row_h * n.b.action_rows as f32;
            let hot = self.hot_action.is_some_and(|h| h.node == index && h.more);
            if hot {
                painter.rect_filled(
                    Rect::from_min_max(pos2(a.x + 3.0 * s, top), pos2(a.x + w - 3.0 * s, top + row_h)),
                    radius(3.0 * s),
                    Palette::alpha(p.hover, text_a),
                );
            }
            let more = format!("+{} more", n.b.actions - n.b.action_rows);
            painter.text(
                pos2(a.x + inset, top + row_h * 0.5),
                Align2::LEFT_CENTER,
                more,
                body.clone(),
                Palette::alpha(p.text_secondary, text_a),
            );
        }
    }

    /// The pins: dots on the edges in their role's colour, the name of a pin that is not the way
    /// on beside it, where its wire starts.
    #[allow(clippy::too_many_arguments)]
    fn draw_pins(
        &mut self,
        painter: &Painter,
        p: &Palette,
        sc: &Scene,
        cx: &CanvasCx<'_>,
        i: usize,
        detail: Detail,
        alpha: f32,
        mouse: Pos2,
        ppp: f32,
    ) {
        let Some(n) = sc.nodes.get(i) else { return };
        let view = &*cx.view;
        let s = Self::scale(view);
        let pr = (2.0 / ppp).max(PIN_R * s);
        let halo = Self::pin_grab(view, ppp);
        let full = detail == Detail::Full;
        let label_font = Self::font(Font::Body, BODY_LINE, view, ppp);
        let by_pin = sc.wires.by_pin;
        let pa = alpha.max(0.5);
        for (k, pin) in n.pins.iter().enumerate() {
            let c = self.output_pin(sc, view, i, k);
            let linking = matches!(&self.linking, Some(Linking::From { node, pin: pn }) if *node == n.id && pn == pin.name.as_str());
            let over = self.hot_node == Some(i) && self.hot_pin == Some(k);
            if full && pin.role != PinRole::Default && !n.trigger {
                let at = match sc.flow {
                    Flow::Right => pos2(c.x + 8.0 * s, c.y - 8.0 * s),
                    Flow::Down => pos2(c.x + 6.0 * s, c.y + 10.0 * s),
                };
                let tw = gz_ui::text::width(painter.ctx(), &label_font, &pin.name).min(160.0 * s);
                let plate = Rect::from_min_size(
                    pos2(at.x - 2.0, at.y - label_font.size * 0.7),
                    vec2(tw + 4.0, label_font.size * 1.4),
                );
                painter.rect_filled(plate, CornerRadius::same(2), Palette::alpha(p.canvas, 0.8));
                paint_fit(
                    painter,
                    &label_font,
                    at,
                    if over || linking {
                        p.pin_hot
                    } else {
                        Palette::alpha(role_color(p, pin.role, by_pin), pa)
                    },
                    &pin.name,
                    160.0 * s,
                );
            }
            if over {
                painter.circle_filled(c, halo, Palette::alpha(p.pin_hot, 0.15));
                let mut t = format!("{}: ", look::pretty(&pin.name));
                t.push_str(if pin.desc.is_empty() { "a way out of this node" } else { &pin.desc });
                t.push_str(if pin.connected { "\nDrag to wire it elsewhere" } else { "\nDrag to wire it" });
                self.tip = t;
            }
            let rr = if over { pr * 1.4 } else { pr };
            let fill =
                if linking || over { p.pin_hot } else { Palette::alpha(pin_color(p, pin.role, by_pin), pa) };
            if pin.connected || linking || over {
                painter.circle_filled(c, rr, fill);
            } else {
                painter.circle_filled(c, rr, p.canvas);
                painter.circle_stroke(c, rr - 0.5, Stroke::new((1.5 * s).max(1.0), fill));
                continue;
            }
            painter.circle_stroke(c, rr, Stroke::new(1.0, p.canvas));
        }
        if !n.trigger {
            let c = self.input_pin(sc, view, i);
            let over = self.hot_node == Some(i) && self.hot_pin.is_none();
            let target = over && (self.linking.is_some() || self.rewire.is_some());
            if over {
                painter.circle_filled(
                    c,
                    halo,
                    Palette::alpha(if target { p.pin_target } else { p.pin_hot }, 0.18),
                );
                if !target && self.tip.is_empty() {
                    self.tip = "The way into this node\nDrag to wire it from a node".to_owned();
                }
            }
            let rr = if over { pr * 1.4 } else { pr };
            let fill = if target {
                p.pin_target
            } else if over {
                p.pin_hot
            } else {
                Palette::alpha(p.pin_input, pa)
            };
            painter.circle_filled(c, rr, fill);
            painter.circle_stroke(c, rr, Stroke::new(1.0, p.canvas));
        }
        let _ = mouse;
    }

    fn draw_guides(&self, painter: &Painter, p: &Palette, view: &ViewMirror) {
        if self.drag.as_ref().is_none_or(|d| !d.moved) {
            return;
        }
        let stroke = Stroke::new(1.0, Palette::alpha(p.accent_bright, 0.8));
        for g in &self.guides {
            let a = self.to_screen(view, [g[0].x, g[0].y]);
            let b = self.to_screen(view, [g[1].x, g[1].y]);
            painter.extend(Shape::dashed_line(&[a, b], stroke, 4.0, 3.0));
        }
    }

    fn draw_marquee(&self, painter: &Painter, p: &Palette, mouse: Pos2) {
        let Some(from) = self.marquee else { return };
        let r = Rect::from_two_pos(from, mouse);
        painter.rect_filled(r, CornerRadius::ZERO, Palette::alpha(p.accent_bright, 0.12));
        painter.rect_stroke(
            r,
            CornerRadius::ZERO,
            Stroke::new(1.0, Palette::alpha(p.accent_bright, 0.8)),
            StrokeKind::Inside,
        );
    }

    /// Ctrl + right drag: the line wires are cut along.
    fn draw_cut(&self, painter: &Painter, p: &Palette, mouse: Pos2) {
        let Some(from) = self.cut else { return };
        painter.extend(Shape::dashed_line(&[from, mouse], Stroke::new(1.5, p.danger), 6.0, 4.0));
        icons::draw(
            painter,
            Icon::Unlink,
            Rect::from_center_size(mouse + vec2(10.0, -10.0), Vec2::splat(14.0)),
            p.danger,
        );
    }

    fn draw_action_drag(
        &mut self,
        painter: &Painter,
        p: &Palette,
        sc: &Scene,
        cx: &CanvasCx<'_>,
        mouse: Pos2,
        ppp: f32,
    ) {
        let Some(d) = self.act_drag.clone().filter(|d| d.dragging) else { return };
        let Some(gn) = cx.graph.find_node(&d.node) else { return };
        let list = if d.exit { &gn.on_exit } else { &gn.on_enter };
        let Some(act) = list.get(d.index) else { return };
        let detail = if Self::full(cx.view, ppp) { Detail::Full } else { Detail::Header };
        let target = self.drop_target(sc, cx.view, cx.graph, mouse, detail);
        if let Some((node, _, _, line)) = target {
            let r = self.rect_of(sc, cx.view, node);
            let s = Self::scale(cx.view);
            painter.line_segment(
                [pos2(r.min.x + 4.0 * s, line), pos2(r.max.x - 4.0 * s, line)],
                Stroke::new(2.0, p.accent_bright),
            );
        }
        // red over anything that is not a place for it: releasing there cancels the move
        let title = look::kind_title(cx.catalog, &act.kind, cx.graph.is_behaviour());
        let w = gz_ui::text::width(painter.ctx(), &Font::Body.id(), &title).min(200.0) + 34.0;
        let pill = Rect::from_min_size(pos2(mouse.x + 12.0, mouse.y + 4.0), vec2(w, 24.0));
        painter.rect_filled(pill, CornerRadius::same(4), Palette::alpha(p.overlay, 0.95));
        painter.rect_stroke(
            pill,
            CornerRadius::same(4),
            Stroke::new(1.0, if target.is_some() { p.accent_bright } else { p.error }),
            StrokeKind::Inside,
        );
        let icon = look::kind_icon(cx.catalog, &act.kind);
        icons::draw(
            painter,
            icon,
            Rect::from_min_size(pos2(pill.min.x + 6.0, pill.center().y - 7.0), Vec2::splat(14.0)),
            look::family_color(p, look::family_of(icon), 1.0),
        );
        paint_fit(
            painter,
            &Font::Body.id(),
            pos2(pill.min.x + 26.0, pill.center().y),
            p.text,
            &title,
            w - 32.0,
        );
    }

    /// The minimap's box: never more than a fifth of the canvas's height, in the bottom right
    /// corner; `None` when there is no room for one.
    pub(super) fn minimap_box(&self) -> Option<(Pos2, f32)> {
        let margin = 8.0;
        let h = MINIMAP_H.min(self.size.y / 5.0);
        if h < 56.0 || self.size.x < MINIMAP_W * (h / MINIMAP_H) * 2.5 {
            return None;
        }
        let k = h / MINIMAP_H;
        let bmax = pos2(self.origin.x + self.size.x - margin, self.origin.y + self.size.y - margin);
        Some((bmax - vec2(MINIMAP_W * k, h), k))
    }

    /// The minimap, only for a graph big enough to get lost in. The picture is the scene's; a
    /// frame scales and moves a copy of it once per place and adds the view and the selection.
    fn draw_minimap(&mut self, painter: &Painter, p: &Palette, sc: &Scene, cx: &CanvasCx<'_>, mouse: Pos2) {
        self.minimap_hovered = false;
        self.minimap_shown = false;
        if !cx.minimap || sc.nodes.len() < doc::MINIMAP_MIN_NODES {
            return;
        }
        let Some((bmin, k)) = self.minimap_box() else { return };
        let Some(mm) = &sc.minimap else { return };
        self.minimap_shown = true;
        let boxr = Rect::from_min_size(bmin, vec2(MINIMAP_W * k, MINIMAP_H * k));
        painter.rect_filled(boxr, CornerRadius::same(6), Palette::alpha(p.overlay, 0.92));
        painter.rect_stroke(boxr, CornerRadius::same(6), Stroke::new(1.0, p.border), StrokeKind::Inside);
        let mesh = match &self.minimap_cache {
            Some((edit, at, m)) if *edit == sc.edit && *at == bmin && self.minimap_k == k => m.clone(),
            _ => {
                let mut m = mm.mesh.clone();
                for v in &mut m.vertices {
                    v.pos = bmin + v.pos.to_vec2() * k;
                }
                let m = Arc::new(m);
                self.minimap_cache = Some((sc.edit, bmin, m.clone()));
                self.minimap_k = k;
                m
            }
        };
        painter.add(Shape::Mesh(mesh));
        let local = |w: [f32; 2]| bmin + mm.to_local(w).to_vec2() * k;
        for id in &cx.view.selection {
            if let Some(n) = sc.node_of(id).and_then(|i| sc.nodes.get(i)) {
                let r = Rect::from_min_max(local(n.pos), local(add(n.pos, n.size)));
                painter.rect_stroke(
                    r.expand(0.5),
                    CornerRadius::same(1),
                    Stroke::new(1.5, p.text),
                    StrokeKind::Outside,
                );
            }
        }
        for b in &cx.graph.canvas.bookmarks {
            if cx.view.is_selected(&b.node) {
                continue;
            }
            if let Some(n) = sc.node_of(&b.node).and_then(|i| sc.nodes.get(i)) {
                let r = Rect::from_min_max(local(n.pos), local(add(n.pos, n.size)));
                painter.rect_stroke(
                    r.expand(0.5),
                    CornerRadius::same(1),
                    Stroke::new(1.5, p.accent_bright),
                    StrokeKind::Outside,
                );
            }
        }
        if let Some(v) = self.viewport(cx.view) {
            let fitted = Rect::from_min_size(bmin + mm.fitted.min.to_vec2() * k, mm.fitted.size() * k);
            let clamp =
                |q: Pos2| pos2(q.x.clamp(fitted.min.x, fitted.max.x), q.y.clamp(fitted.min.y, fitted.max.y));
            let vr = Rect::from_min_max(clamp(local([v[0], v[1]])), clamp(local([v[2], v[3]])));
            painter.rect_stroke(
                vr,
                CornerRadius::ZERO,
                Stroke::new(1.5, p.accent_bright),
                StrokeKind::Middle,
            );
        }
        self.minimap_hovered = self.hovered && boxr.contains(mouse);
        if self.minimap_hovered {
            self.tip = "Minimap: click or drag to move the view".to_owned();
        }
    }

    /// Where the graph goes on past the view, when no minimap says it: a pill at the middle of each
    /// edge with the nodes wholly beyond it. The counts are kept either way.
    fn draw_beyond(&mut self, painter: &Painter, p: &Palette, sc: &Scene, view: &ViewMirror, mouse: Pos2) {
        self.beyond_hot = None;
        self.beyond = [0; 4];
        self.beyond_near = [None; 4];
        let Some(v) = self.viewport(view) else { return };
        if sc.nodes.is_empty() {
            return;
        }
        let (counts, near) = sc.beyond(v);
        self.beyond = counts;
        self.beyond_near = near;
        if self.minimap_shown {
            return;
        }
        const ICONS: [Icon; 4] = [Icon::ChevronUp, Icon::ChevronDown, Icon::ChevronLeft, Icon::ChevronRight];
        const WHERE: [&str; 4] = ["above", "below", "left of", "right of"];
        let font = Font::Small.id();
        let icon = 12.0;
        let (padx, pady) = (8.0, 4.0);
        let inset = 8.0;
        for side in 0..4 {
            let count = counts.get(side).copied().unwrap_or(0);
            if count == 0 {
                continue;
            }
            let label = format!("{count} more");
            let h = round(Font::Small.line().max(icon) + pady * 2.0);
            let w = round(padx + icon + 4.0 + gz_ui::text::width(painter.ctx(), &font, &label) + padx);
            let a = match side {
                0 => pos2(self.origin.x + (self.size.x - w) * 0.5, self.origin.y + inset),
                1 => pos2(self.origin.x + (self.size.x - w) * 0.5, self.origin.y + self.size.y - inset - h),
                2 => pos2(self.origin.x + inset, self.origin.y + (self.size.y - h) * 0.5),
                _ => pos2(self.origin.x + self.size.x - inset - w, self.origin.y + (self.size.y - h) * 0.5),
            };
            let r = Rect::from_min_size(pos2(round(a.x), round(a.y)), vec2(w, h));
            let hot = self.hovered && r.contains(mouse);
            if hot {
                self.beyond_hot = Some(side);
            }
            painter.rect_filled(r, CornerRadius::same(6), Palette::alpha(p.overlay, 0.92));
            if hot {
                painter.rect_filled(r, CornerRadius::same(6), p.hover);
            }
            painter.rect_stroke(
                r,
                CornerRadius::same(6),
                Stroke::new(1.0, p.border_subtle),
                StrokeKind::Inside,
            );
            if let Some(ic) = ICONS.get(side) {
                icons::draw(
                    painter,
                    *ic,
                    Rect::from_min_size(pos2(r.min.x + padx, r.center().y - icon * 0.5), Vec2::splat(icon)),
                    p.text_secondary,
                );
            }
            painter.text(
                pos2(r.min.x + padx + icon + 4.0, r.center().y),
                Align2::LEFT_CENTER,
                label,
                font.clone(),
                p.text_secondary,
            );
            if hot {
                self.tip = format!(
                    "{count} {} {} the view. Click to bring the nearest one in",
                    if count == 1 { "node" } else { "nodes" },
                    WHERE.get(side).copied().unwrap_or("")
                );
            }
        }
    }

    fn draw_status(&mut self, painter: &Painter, p: &Palette, ctx: &egui::Context) {
        if self.status.is_empty() {
            return;
        }
        let now = ctx.input(|i| i.time);
        if now - self.status_at > STATUS_SECONDS {
            self.status.clear();
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
        let font = Font::Body.id();
        let (shown, _) =
            gz_ui::text::ellipsize(ctx, &font, &self.status, self.size.x - 40.0, gz_ui::Clip::End);
        let w = gz_ui::text::width(ctx, &font, &shown);
        let r = Rect::from_min_size(self.origin + vec2(8.0, 8.0), vec2(w + 20.0, Font::Body.line() + 8.0));
        painter.rect_filled(r, CornerRadius::same(6), Palette::alpha(p.overlay, 0.92));
        painter.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
        painter.text(pos2(r.min.x + 10.0, r.center().y), Align2::LEFT_CENTER, shown, font, p.text_secondary);
    }
}

/// Diagonal lines across a rectangle: a disabled node's header.
fn hatch(painter: &Painter, r: Rect, color: Color32, s: f32) {
    let step = (8.0 * s).max(5.0);
    let clip = painter.with_clip_rect(r.intersect(painter.clip_rect()));
    let stroke = Stroke::new(1.0, Palette::alpha(color, 0.45));
    let mut x = r.min.x - r.height();
    while x < r.max.x {
        clip.line_segment([pos2(x, r.max.y), pos2(x + r.height(), r.min.y)], stroke);
        x += step;
    }
}

/// What a node too small to read says on hover.
fn node_tip(n: &SceneNode, marks: Option<&crate::derived::NodeMarks>) -> String {
    let mut t = format!("{}\n{}", n.title, n.id);
    if !n.summary.is_empty() {
        t.push('\n');
        t.push_str(&n.summary);
    }
    if n.disabled {
        t.push_str("\nDisabled: the game passes through it");
    }
    if let Some(m) = marks {
        t.push('\n');
        t.push_str(&m.first);
    }
    t
}
