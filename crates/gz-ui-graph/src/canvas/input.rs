//! The canvas's gestures: pan (right or middle drag), zoom at the cursor, select and drag nodes
//! (snapped to the grid, with alignment guides; a lone node dropped on a wire goes on it), draw a
//! wire from an output or into an input, move a wire's end (Alt + drag), cut wires (Ctrl + right
//! drag), move and resize comment frames, add, drag and remove reroutes, move actions, the
//! marquee; and what the keys and the clipboard do.

use super::*;

/// The arc length along a polyline of the point nearest `p`.
pub(super) fn arc_of(points: &[[f32; 2]], p: [f32; 2]) -> f32 {
    let (mut best, mut at, mut run) = (f32::MAX, 0.0, 0.0);
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let d = [b[0] - a[0], b[1] - a[1]];
        let len2 = d[0] * d[0] + d[1] * d[1];
        let len = len2.sqrt();
        let t = if len2 > 0.0 {
            (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let q = [a[0] + d[0] * t, a[1] + d[1] * t];
        let dist = (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2);
        if dist < best {
            best = dist;
            at = run + len * t;
        }
        run += len;
    }
    at
}

fn wire_value(from: &str, pin: &str, to: &str) -> Value {
    op_table(&[("from", Value::from(from)), ("pin", Value::from(pin)), ("to", Value::from(to))])
}

impl Canvas {
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    pub(super) fn handle_input(
        &mut self,
        ctx: &egui::Context,
        sc: &Scene,
        cx: &mut CanvasCx<'_>,
        mouse: Pos2,
        grab: f32,
        resp: &egui::Response,
        menu_up: bool,
        ppp: f32,
    ) {
        let (pressed, down, sec_pressed, sec_down, sec_released, mid_down, delta, mods, wheel, now) = ctx
            .input(|i| {
                (
                    i.pointer.primary_pressed(),
                    i.pointer.primary_down(),
                    i.pointer.button_pressed(PointerButton::Secondary),
                    i.pointer.button_down(PointerButton::Secondary),
                    i.pointer.button_released(PointerButton::Secondary),
                    i.pointer.button_down(PointerButton::Middle),
                    i.pointer.delta(),
                    i.modifiers,
                    i.events
                        .iter()
                        .map(|e| match e {
                            egui::Event::MouseWheel { delta, .. } => delta.y,
                            _ => 0.0,
                        })
                        .sum::<f32>(),
                    i.time,
                )
            });
        // a frame that waited (a busy machine) can hold both presses of a double click
        let presses = ctx.input(|i| {
            i.events
                .iter()
                .filter(|e| {
                    matches!(
                        e,
                        egui::Event::PointerButton { button: PointerButton::Primary, pressed: true, .. }
                    )
                })
                .count()
        });
        // a second press near the first one soon after it is a double click, told at the press:
        // egui tells one at the release, when the first press already started a gesture
        let double = pressed && {
            let (delay, dist) =
                ctx.options(|o| (o.input_options.max_double_click_delay, o.input_options.max_click_dist));
            let twice = presses >= 2
                || self.last_press.is_some_and(|(t, at)| now - t <= delay && (mouse - at).length() <= dist);
            self.last_press = if twice { None } else { Some((now, mouse)) };
            twice
        };
        let covered = cx.covered.iter().any(|r| r.contains(mouse));
        // a right press beside the canvas's own menu takes it down and is the canvas's press: the
        // menu of what is under it comes up at the release, as with no menu up
        let mut menu_up = menu_up;
        if sec_pressed
            && menu_up
            && self.frame_title.is_none()
            && egui::Popup::is_id_open(ctx, self.id.with("menu"))
            && resp.contains_pointer()
            && !covered
        {
            self.close_menu(ctx);
            menu_up = egui::Popup::is_any_open(ctx);
        }
        let over = (resp.hovered() || resp.contains_pointer()) && !menu_up && !covered;
        // the minimap and the edge pills are the canvas's own controls: a click there is theirs
        let hovered = over && !self.minimap_hovered && self.beyond_hot.is_none();

        if over && pressed {
            if let Some(side) = self.beyond_hot {
                self.reveal_beyond(sc, cx.view, side);
            }
            if self.minimap_hovered {
                self.minimap_panning = true;
            }
        }
        if self.minimap_panning {
            if down {
                if let (Some(mm), Some((bmin, k))) = (&sc.minimap, self.minimap_box()) {
                    let w = mm.to_world(((mouse - bmin) / k).to_pos2());
                    cx.view.center = [layout::sane(w[0]), layout::sane(w[1])];
                    self.cancel_framing();
                }
            } else {
                self.minimap_panning = false;
            }
        }

        // the wheel zooms around the cursor
        if hovered && wheel != 0.0 {
            self.zoom_by(cx.view, if wheel > 0.0 { 1 } else { -1 }, Some(mouse));
            self.cancel_framing();
            self.last_wheel = now;
        }

        // the right button: Ctrl cuts wires, else it pans while dragging and opens the menu of what
        // is under it on a click
        if hovered && sec_pressed {
            if mods.ctrl || mods.command {
                self.cut = Some(mouse);
            } else {
                self.pan = Some((mouse, false));
            }
        }
        if let Some(start) = self.cut
            && (sec_released || !sec_down)
        {
            self.cut = None;
            self.end_cut(ctx, sc, cx, start, mouse);
        }
        if let Some((start, moved)) = self.pan {
            if sec_down {
                if !moved && (mouse.x - start.x).abs() + (mouse.y - start.y).abs() > 3.0 {
                    self.pan = Some((start, true));
                }
                if self.pan.is_some_and(|(_, m)| m) && delta != Vec2::ZERO {
                    let s = Self::scale(cx.view);
                    cx.view.center = [cx.view.center[0] - delta.x / s, cx.view.center[1] - delta.y / s];
                    self.cancel_framing();
                }
            }
            if sec_released || !sec_down {
                self.pan = None;
                if !moved && hovered {
                    self.open_context_menu(ctx, sc, cx, mouse, ppp);
                }
            }
        }
        // the middle button pans too
        if hovered && mid_down && delta != Vec2::ZERO {
            self.middle_pan = true;
            let s = Self::scale(cx.view);
            cx.view.center = [cx.view.center[0] - delta.x / s, cx.view.center[1] - delta.y / s];
            self.cancel_framing();
        }
        if !mid_down {
            self.middle_pan = false;
        }

        if hovered && pressed {
            self.press(sc, cx, mouse, mods, double, ppp);
        }
        self.follow_gestures(ctx, sc, cx, mouse, grab, down, ppp);
        if !down && let Some(r) = self.on_release.take() {
            match r {
                OnRelease::Rename => self.want_rename = true,
                OnRelease::FrameTitle(fi, title) => self.frame_title = Some((fi, title, true)),
            }
        }

        // what the view does is the document's once the gesture is over
        let wheel_busy = now - self.last_wheel < WHEEL_SETTLE;
        self.view_moving =
            self.pan.is_some_and(|(_, m)| m) || self.middle_pan || self.minimap_panning || wheel_busy;
        if wheel_busy {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
        self.hover_tip(ctx, sc, cx, mouse, hovered);
    }

    /// The left button went down on the canvas.
    #[allow(clippy::too_many_lines)]
    fn press(
        &mut self,
        sc: &Scene,
        cx: &mut CanvasCx<'_>,
        mouse: Pos2,
        mods: egui::Modifiers,
        double: bool,
        ppp: f32,
    ) {
        // a reroute: a click selects it (Ctrl or Shift adds it or takes it out), a drag moves it with
        // the rest of the selection, a double click takes it away
        if let Some((li, k)) = self.hot_knot {
            if double {
                if let Some(l) = sc.links.get(li) {
                    let mut via = l.via.clone();
                    if k < via.len() {
                        via.remove(k);
                    }
                    self.forget_knot(sc, li, k);
                    self.push_route(sc, cx, li, &via);
                }
                return;
            }
            let Some(r) = Self::knot_ref(sc, li, k) else { return };
            let picked = self.knots.iter().any(|x| x.same(&r));
            self.knots_kept = true;
            if mods.ctrl || mods.command || mods.shift {
                if picked {
                    self.knots.retain(|x| !x.same(&r));
                    self.knot_hits.retain(|x| *x != (li, k));
                    return;
                }
                self.knots.push(r.clone());
                self.knot_hits.push((li, k));
            } else if !picked {
                cx.view.selection.clear();
                cx.view.slot.clear();
                self.knots = vec![r.clone()];
                self.knot_hits = vec![(li, k)];
            }
            let mut d = self.group_drag(sc, cx.view, None, mouse);
            let plain = !(mods.ctrl || mods.command || mods.shift);
            if plain && picked && (self.knots.len() + cx.view.selection.len()) > 1 {
                d.single = Some(Single::Knot(r));
            }
            self.drag = Some(d);
            return;
        }
        // Alt on a wire: a click removes it, a drag moves its nearer end
        if mods.alt
            && let Some(li) = self.hot_link
        {
            self.alt_press = Some((li, mouse));
            return;
        }
        // a pin starts a wire: out of an output, into an input
        if let Some(n) = self.hot_node
            && let Some(sn) = sc.nodes.get(n)
        {
            self.linking = Some(match self.hot_pin.and_then(|k| sn.pins.get(k)) {
                Some(pin) => Linking::From { node: sn.id.clone(), pin: SmolStr::new(&pin.name) },
                None => Linking::Into { node: sn.id.clone() },
            });
            self.link_press = mouse;
            return;
        }
        if let Some(n) = self.hit_node(sc, cx.view, mouse) {
            let Some(sn) = sc.nodes.get(n) else { return };
            let detail = Self::detail(cx.view, ppp, sn.size[0]);
            if let Some(hit) = self.hit_action(sc, cx.view, n, mouse, detail) {
                cx.view.select_only(&sn.id);
                if hit.more {
                    // the first action the node has no room for opens in the Details panel
                    if let Some((exit, index)) = sn.more_target {
                        cx.view.slot = format!("{}:{index}", slot_name(exit));
                    }
                    self.want_focus_action = true;
                } else if let Some(row) = sn.action_at(hit.row) {
                    cx.view.slot = format!("{}:{}", slot_name(row.exit), row.index);
                    self.act_drag = Some(ActDrag {
                        node: sn.id.clone(),
                        exit: row.exit,
                        index: row.index,
                        start: mouse,
                        dragging: false,
                    });
                    if double {
                        self.want_focus_action = true;
                    }
                }
                return;
            }
            let id = sn.id.clone();
            if double && self.hit_header(sc, cx.view, n, mouse) {
                cx.view.select_only(&id);
                self.on_release = Some(OnRelease::Rename);
                return;
            }
            let mut single = None;
            if mods.ctrl || mods.command || mods.shift {
                cx.view.toggle(&id);
                self.knots_kept = true;
            } else if !cx.view.is_selected(&id) {
                cx.view.select_only(&id);
                self.clear_knots();
            } else {
                cx.view.slot.clear();
                if cx.view.selection.len() + self.knots.len() > 1 {
                    single = Some(Single::Node(id.clone()));
                }
            }
            if cx.view.is_selected(&id) {
                let mut d = self.group_drag(sc, cx.view, Some(n), mouse);
                d.single = single;
                self.drag = Some(d);
            }
            return;
        }
        if let Some((fi, resize)) = self.hot_frame {
            if double && !resize {
                let title =
                    sc.frames.iter().find(|f| f.index == fi).map(|f| f.title.clone()).unwrap_or_default();
                self.on_release = Some(OnRelease::FrameTitle(fi, title));
                return;
            }
            let inside = sc
                .frames
                .iter()
                .find(|f| f.index == fi)
                .map(|f| {
                    let r = f.rect;
                    sc.nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, n)| {
                            let b = n.rect();
                            b[0] >= r[0] && b[1] >= r[1] && b[2] <= r[2] && b[3] <= r[3]
                        })
                        .map(|(i, _)| i)
                        .collect()
                })
                .unwrap_or_default();
            self.frame_drag = Some(FrameDrag { index: fi, start: mouse, resize, delta: [0.0, 0.0], inside });
            return;
        }
        // a double click on a wire puts a reroute there
        if double && let Some(li) = self.hot_link {
            if let Some(l) = sc.links.get(li) {
                let w = self.to_world(cx.view, mouse);
                let t = arc_of(&l.points, w);
                let k = l.via.iter().filter(|v| arc_of(&l.points, **v) < t).count();
                let mut via = l.via.clone();
                via.insert(k, [layout::sane(w[0]), layout::sane(w[1])]);
                self.push_route(sc, cx, li, &via);
            }
            return;
        }
        self.marquee = Some(mouse);
        if !(mods.ctrl || mods.command || mods.shift) {
            cx.view.selection.clear();
            cx.view.slot.clear();
            self.clear_knots();
        }
    }

    /// A drag of the selection, nodes and reroutes, from the press at `mouse`; `node` is the node
    /// pressed (the one the grid and the guides place), none for a reroute.
    fn group_drag(&self, sc: &Scene, view: &ViewMirror, node: Option<usize>, mouse: Pos2) -> Drag {
        let nodes: Vec<(usize, SmolStr, [f32; 2])> = view
            .selection
            .iter()
            .filter_map(|s| sc.node_of(s).map(|i| (i, s.clone(), self.pos_of(sc, i))))
            .collect();
        // the node pressed places the drag on the grid; pressed on a reroute, the first node does
        let anchor = match node {
            Some(n) => nodes.iter().position(|(i, _, _)| *i == n).unwrap_or(0),
            None if nodes.is_empty() => usize::MAX,
            None => 0,
        };
        let knots: Vec<(usize, usize, [f32; 2])> =
            self.knot_hits.iter().map(|&(li, k)| (li, k, self.knot_at(sc, li, k))).collect();
        let knot_refs = knots.iter().filter_map(|&(li, k, _)| Self::knot_ref(sc, li, k)).collect();
        Drag {
            nodes,
            knots,
            knot_refs,
            edit: sc.edit,
            anchor,
            start: mouse,
            moved: false,
            delta: [0.0, 0.0],
            on_wire: None,
            single: None,
        }
    }

    /// The gestures under way: each follows the pointer while the button is down and ends when it
    /// is let go.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn follow_gestures(
        &mut self,
        ctx: &egui::Context,
        sc: &Scene,
        cx: &mut CanvasCx<'_>,
        mouse: Pos2,
        grab: f32,
        down: bool,
        ppp: f32,
    ) {
        let s = Self::scale(cx.view);
        // Alt on a wire: moved, it becomes a rewire of the nearer end; let go, the wire goes
        if let Some((li, start)) = self.alt_press {
            if down && (mouse - start).length() > 4.0 {
                self.alt_press = None;
                if let Some(l) = sc.links.get(li)
                    && let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize))
                {
                    let pts = self.link_points(sc, li);
                    let a = pts.first().map_or(start, |q| self.to_screen(cx.view, *q));
                    let b = pts.last().map_or(start, |q| self.to_screen(cx.view, *q));
                    self.rewire = Some(Rewire {
                        from: f.id.clone(),
                        pin: l.pin.clone(),
                        to: t.id.clone(),
                        target: (start - b).length() <= (start - a).length(),
                    });
                }
            } else if !down {
                self.alt_press = None;
                self.remove_wire(ctx, sc, cx, li);
            }
        }
        // nodes being dragged
        if let Some(mut d) = self.drag.take() {
            if down {
                let dd = mouse - d.start;
                if dd.x.abs() + dd.y.abs() > 2.0 {
                    d.moved = true;
                }
                if d.moved {
                    d.delta = self.adjust_drag(sc, cx, &d, [dd.x / s, dd.y / s]);
                    d.on_wire = self.wire_for_lone(sc, cx, &d, mouse);
                }
                self.drag = Some(d);
            } else {
                self.end_drag(ctx, sc, cx, d);
            }
        }
        // a frame being moved or resized
        if let Some(mut f) = self.frame_drag.take() {
            if down {
                let dd = mouse - f.start;
                let raw = [dd.x / s, dd.y / s];
                f.delta = if cx.prefs.snap { [layout::snap(raw[0]), layout::snap(raw[1])] } else { raw };
                self.frame_drag = Some(f);
            } else {
                self.end_frame_drag(sc, cx, f);
            }
        }
        // an action being dragged: a press only nominates it, the move starts once the mouse travelled
        let mut act_over = None;
        if let Some(a) = &mut self.act_drag {
            if down && (mouse - a.start).length() > 6.0 {
                a.dragging = true;
            }
            if !down {
                act_over = Some(a.clone());
            }
        }
        if let Some(a) = act_over {
            self.act_drag = None;
            let detail = if Self::full(cx.view, ppp) { Detail::Full } else { Detail::Header };
            if a.dragging
                && let Some((node, exit, at, _)) = self.drop_target(sc, cx.view, cx.graph, mouse, detail)
                && let Some(to) = sc.nodes.get(node)
            {
                move_action(cx, &a.node, a.exit, a.index, &to.id, exit, at);
            }
        }
        if self.linking.is_some() && !down {
            self.end_link(ctx, sc, cx, mouse, grab);
        }
        if self.rewire.is_some() && !down {
            self.end_rewire(ctx, sc, cx, mouse, grab);
        }
        if let Some(from) = self.marquee
            && !down
        {
            self.marquee = None;
            let r = Rect::from_two_pos(from, mouse);
            if r.width() >= 2.0 || r.height() >= 2.0 {
                let a = self.to_world(cx.view, r.min);
                let b = self.to_world(cx.view, r.max);
                self.scratch.clear();
                let mut scratch = std::mem::take(&mut self.scratch);
                sc.nodes_in([a[0], a[1], b[0], b[1]], &mut scratch);
                let found = self.dedupe(sc.nodes.len(), &scratch);
                self.scratch = scratch;
                for i in found {
                    if !self.rect_of(sc, cx.view, i).intersects(r) {
                        continue;
                    }
                    if let Some(n) = sc.nodes.get(i)
                        && !cx.view.is_selected(&n.id)
                    {
                        cx.view.selection.push(n.id.clone());
                    }
                }
                // the reroutes inside the rectangle
                self.knots_kept = true;
                for li in self.visible_links(sc, [a[0], a[1], b[0], b[1]]) {
                    let Some(l) = sc.links.get(li) else { continue };
                    for (k, v) in l.via.iter().enumerate() {
                        if v[0] >= a[0]
                            && v[0] <= b[0]
                            && v[1] >= a[1]
                            && v[1] <= b[1]
                            && let Some(kr) = Self::knot_ref(sc, li, k)
                            && !self.knots.iter().any(|x| x.same(&kr))
                        {
                            self.knots.push(kr);
                            self.knot_hits.push((li, k));
                        }
                    }
                }
            }
        }
    }

    /// A drag's offset after the grid and the alignment guides: the node under the pointer lands
    /// on the grid, or on a neighbour's edge or centre when one is within a few px.
    fn adjust_drag(&mut self, sc: &Scene, cx: &CanvasCx<'_>, d: &Drag, raw: [f32; 2]) -> [f32; 2] {
        self.guides.clear();
        let Some(&(ai, _, start)) = d.nodes.get(d.anchor) else {
            // dragged by a reroute: a lone reroute lands on the grid
            return match d.knots.as_slice() {
                [(_, _, at)] if d.nodes.is_empty() && cx.prefs.snap => {
                    let to = add(*at, raw);
                    [layout::snap(to[0]) - at[0], layout::snap(to[1]) - at[1]]
                }
                _ => raw,
            };
        };
        let Some(an) = sc.nodes.get(ai) else { return raw };
        let mut at = add(start, raw);
        if cx.prefs.snap {
            at = [layout::snap(at[0]), layout::snap(at[1])];
        }
        if !cx.prefs.guides {
            return [at[0] - start[0], at[1] - start[1]];
        }
        let tol = GUIDE_PX / Self::scale(cx.view);
        let Some(v) = self.viewport(cx.view) else { return [at[0] - start[0], at[1] - start[1]] };
        self.scratch.clear();
        let mut scratch = std::mem::take(&mut self.scratch);
        sc.nodes_in(v, &mut scratch);
        let near = self.dedupe(sc.nodes.len(), &scratch);
        self.scratch = scratch;
        let (w, h) = (an.size[0], an.size[1]);
        // per axis: the best (difference, the neighbour's edge, the neighbour)
        let mut best_x: Option<(f32, f32, usize)> = None;
        let mut best_y: Option<(f32, f32, usize)> = None;
        for j in near {
            if d.nodes.iter().any(|(i, _, _)| *i == j) {
                continue;
            }
            let Some(nb) = sc.nodes.get(j) else { continue };
            let (nx, ny, nw, nh) = (nb.pos[0], nb.pos[1], nb.size[0], nb.size[1]);
            for (mine, off) in [(at[0], 0.0), (at[0] + w * 0.5, w * 0.5), (at[0] + w, w)] {
                for edge in [nx, nx + nw * 0.5, nx + nw] {
                    let diff = edge - mine;
                    if diff.abs() <= tol && best_x.is_none_or(|(b, _, _)| diff.abs() < b.abs()) {
                        best_x = Some((diff, edge - off, j));
                    }
                }
            }
            for (mine, off) in [(at[1], 0.0), (at[1] + h * 0.5, h * 0.5), (at[1] + h, h)] {
                for edge in [ny, ny + nh * 0.5, ny + nh] {
                    let diff = edge - mine;
                    if diff.abs() <= tol && best_y.is_none_or(|(b, _, _)| diff.abs() < b.abs()) {
                        best_y = Some((diff, edge - off, j));
                    }
                }
            }
        }
        if let Some((diff, _, j)) = best_x {
            at[0] += diff;
            if let Some(nb) = sc.nodes.get(j) {
                let x = [at[0], at[0] + w * 0.5, at[0] + w]
                    .into_iter()
                    .find(|x| {
                        [nb.pos[0], nb.pos[0] + nb.size[0] * 0.5, nb.pos[0] + nb.size[0]]
                            .iter()
                            .any(|e| (e - x).abs() < 0.01)
                    })
                    .unwrap_or(at[0]);
                let (y0, y1) = (at[1].min(nb.pos[1]), (at[1] + h).max(nb.pos[1] + nb.size[1]));
                self.guides.push([pos2(x, y0), pos2(x, y1)]);
            }
        }
        if let Some((diff, _, j)) = best_y {
            at[1] += diff;
            if let Some(nb) = sc.nodes.get(j) {
                let y = [at[1], at[1] + h * 0.5, at[1] + h]
                    .into_iter()
                    .find(|y| {
                        [nb.pos[1], nb.pos[1] + nb.size[1] * 0.5, nb.pos[1] + nb.size[1]]
                            .iter()
                            .any(|e| (e - y).abs() < 0.01)
                    })
                    .unwrap_or(at[1]);
                let (x0, x1) = (at[0].min(nb.pos[0]), (at[0] + w).max(nb.pos[0] + nb.size[0]));
                self.guides.push([pos2(x0, y), pos2(x1, y)]);
            }
        }
        [at[0] - start[0], at[1] - start[1]]
    }

    /// The wire a lone node is held over: a node with no wire of its own that can take one in and
    /// go on goes on the wire it is dropped on.
    fn wire_for_lone(&mut self, sc: &Scene, cx: &CanvasCx<'_>, d: &Drag, mouse: Pos2) -> Option<usize> {
        let [(i, _, _)] = d.nodes.as_slice() else { return None };
        if !d.knots.is_empty() {
            return None;
        }
        let n = sc.nodes.get(*i)?;
        let wired = n.trigger
            || n.pins.iter().any(|p| p.connected)
            || !n.pins.iter().any(|p| p.role == PinRole::Default)
            || sc.links.iter().any(|l| l.to as usize == *i);
        if wired {
            return None;
        }
        self.hit_link(sc, cx.view, mouse)
    }

    fn end_drag(&mut self, ctx: &egui::Context, sc: &Scene, cx: &mut CanvasCx<'_>, d: Drag) {
        self.guides.clear();
        if !d.moved {
            // a click on one of several selected things singles it out
            match d.single {
                Some(Single::Node(id)) => {
                    cx.view.select_only(&id);
                    self.clear_knots();
                }
                Some(Single::Knot(r)) => {
                    cx.view.selection.clear();
                    cx.view.slot.clear();
                    self.knots = vec![r];
                    self.knots_kept = true;
                    self.resolve_knots(sc);
                }
                None => {}
            }
            return;
        }
        if !d.knots.is_empty() {
            self.end_group_drag(ctx, sc, cx, &d);
            return;
        }
        let at: Vec<(usize, SmolStr, [f32; 2])> = d
            .nodes
            .iter()
            .map(|(i, id, start)| {
                (*i, id.clone(), [layout::sane(start[0] + d.delta[0]), layout::sane(start[1] + d.delta[1])])
            })
            .collect();
        match (d.on_wire, at.as_slice(), cx.scene) {
            (Some(li), [(_, id, p)], Some(sc)) => {
                let Some(l) = sc.links.get(li) else { return };
                let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) else {
                    return;
                };
                let ops = vec![
                    op_table(&[
                        ("op", Value::from("move")),
                        ("id", Value::from(id.as_str())),
                        ("x", Value::from(f64::from(p[0]))),
                        ("y", Value::from(f64::from(p[1]))),
                    ]),
                    op_table(&[
                        ("op", Value::from("insert_on_wire")),
                        ("from", Value::from(f.id.as_str())),
                        ("pin", Value::from(l.pin.as_str())),
                        ("to", Value::from(t.id.as_str())),
                        ("existing", Value::from(id.as_str())),
                    ]),
                ];
                cx.ops.push(Op::Doc(DocOp::ApplyOps(Value::from(ops))));
            }
            _ => {
                let label = if at.len() == 1 { "Move Node" } else { "Move Nodes" };
                cx.ops.push(Op::Doc(DocOp::Positions {
                    at: at.iter().map(|(_, id, p)| (id.clone(), *p)).collect(),
                    label,
                }));
            }
        }
        self.settle = Some(Settle {
            from_edit: cx.scene.map_or(cx.edit, |s| s.edit),
            at: at.into_iter().map(|(i, _, p)| (i, p)).collect(),
            knots: Vec::new(),
        });
    }

    /// A drag that moved reroutes: the nodes' places and the wires' points, one undo step.
    fn end_group_drag(&mut self, ctx: &egui::Context, sc: &Scene, cx: &mut CanvasCx<'_>, d: &Drag) {
        // a scene that came during the drag (an undo, the file changed outside) numbers its wires
        // anew: the reroutes are found again there by their wires and points, or nothing moves
        let mut places = d.knots.clone();
        if sc.edit != d.edit {
            places.clear();
            for (r, (_, _, start)) in d.knot_refs.iter().zip(&d.knots) {
                let Some((li, k)) = Self::find_knot(sc, r) else {
                    self.set_status(ctx, "The graph changed during the drag: nothing was moved");
                    return;
                };
                places.push((li, k, *start));
            }
        }
        let moved = |p: [f32; 2]| [layout::sane(p[0] + d.delta[0]), layout::sane(p[1] + d.delta[1])];
        let mut ops: Vec<Value> = Vec::new();
        let mut at = Vec::with_capacity(d.nodes.len());
        for (i, id, start) in &d.nodes {
            let p = moved(*start);
            ops.push(op_table(&[
                ("op", Value::from("move")),
                ("id", Value::from(id.as_str())),
                ("x", Value::from(f64::from(p[0]))),
                ("y", Value::from(f64::from(p[1]))),
            ]));
            at.push((*i, p));
        }
        let mut wires: Vec<usize> = places.iter().map(|(li, _, _)| *li).collect();
        wires.sort_unstable();
        wires.dedup();
        let mut knots = Vec::with_capacity(places.len());
        for li in wires {
            let Some(l) = sc.links.get(li) else { continue };
            let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) else {
                continue;
            };
            let mut via = l.via.clone();
            for (_, k, start) in places.iter().filter(|(w, _, _)| *w == li) {
                if let Some(v) = via.get_mut(*k) {
                    let was = *v;
                    *v = moved(*start);
                    knots.push((li, *k, *v));
                    // the selection follows the reroute to its new point
                    if let Some(r) = Self::knot_ref(sc, li, *k) {
                        self.knot_moved(&KnotRef { at: was, ..r }, *v);
                    }
                }
            }
            ops.push(op_table(&[
                ("op", Value::from("route")),
                ("from", Value::from(f.id.as_str())),
                ("pin", Value::from(l.pin.as_str())),
                ("to", Value::from(t.id.as_str())),
                ("points", Value::from(via.iter().map(|q| point(*q)).collect::<Vec<_>>())),
            ]));
        }
        if ops.is_empty() {
            return;
        }
        cx.ops.push(Op::Doc(DocOp::ApplyOps(Value::from(ops))));
        self.settle = Some(Settle { from_edit: sc.edit, at, knots });
    }

    fn end_frame_drag(&mut self, sc: &Scene, cx: &mut CanvasCx<'_>, f: FrameDrag) {
        #[allow(clippy::float_cmp)]
        if f.delta == [0.0, 0.0] {
            return;
        }
        let Some(fr) = sc.frames.iter().find(|x| x.index == f.index) else { return };
        let index = Value::from((f.index + 1) as f64);
        let r = fr.rect;
        let to = if f.resize {
            [r[0], r[1], (r[2] + f.delta[0]).max(r[0] + 64.0), (r[3] + f.delta[1]).max(r[1] + 48.0)]
        } else {
            [r[0] + f.delta[0], r[1] + f.delta[1], r[2] + f.delta[0], r[3] + f.delta[1]]
        };
        self.frame_settle = Some((sc.edit, f.index, to));
        if !f.resize {
            // the selected reroutes of the wires it holds whole follow them
            for (li, l) in sc.links.iter().enumerate() {
                if f.inside.contains(&(l.from as usize)) && f.inside.contains(&(l.to as usize)) {
                    for k in 0..l.via.len() {
                        if let Some(r) = Self::knot_ref(sc, li, k) {
                            let now = add(r.at, f.delta);
                            self.knot_moved(&r, now);
                        }
                    }
                }
            }
        }
        let op = if f.resize {
            op_table(&[
                ("op", Value::from("frame")),
                ("action", Value::from("set")),
                ("index", index),
                ("w", Value::from(f64::from((r[2] - r[0] + f.delta[0]).max(64.0)))),
                ("h", Value::from(f64::from((r[3] - r[1] + f.delta[1]).max(48.0)))),
            ])
        } else {
            self.settle = Some(Settle {
                from_edit: sc.edit,
                at: f
                    .inside
                    .iter()
                    .filter_map(|&i| sc.nodes.get(i).map(|n| (i, add(n.pos, f.delta))))
                    .collect(),
                // the reroutes of the wires it holds whole
                knots: sc
                    .links
                    .iter()
                    .enumerate()
                    .filter(|(_, l)| {
                        f.inside.contains(&(l.from as usize)) && f.inside.contains(&(l.to as usize))
                    })
                    .flat_map(|(li, l)| l.via.iter().enumerate().map(move |(k, v)| (li, k, add(*v, f.delta))))
                    .collect(),
            });
            op_table(&[
                ("op", Value::from("frame")),
                ("action", Value::from("move")),
                ("index", index),
                ("by", point(f.delta)),
            ])
        };
        cx.ops.push(Op::Doc(DocOp::Apply(op)));
    }

    /// A wire's reroutes set again.
    pub(super) fn push_route(&self, sc: &Scene, cx: &mut CanvasCx<'_>, li: usize, via: &[[f32; 2]]) {
        let Some(l) = sc.links.get(li) else { return };
        let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) else { return };
        let points: Vec<Value> = via.iter().map(|q| point(*q)).collect();
        cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
            ("op", Value::from("route")),
            ("from", Value::from(f.id.as_str())),
            ("pin", Value::from(l.pin.as_str())),
            ("to", Value::from(t.id.as_str())),
            ("points", Value::from(points)),
        ]))));
    }

    /// Removes one wire, saying so.
    pub(super) fn remove_wire(&mut self, ctx: &egui::Context, sc: &Scene, cx: &mut CanvasCx<'_>, li: usize) {
        let Some(l) = sc.links.get(li) else { return };
        let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) else { return };
        cx.ops.push(Op::Doc(DocOp::Disconnect {
            from: f.id.to_string(),
            pin: l.pin.to_string(),
            to: t.id.to_string(),
        }));
        let msg = gz_ui::tr!(
            "Removed the wire {from}.{pin} -> {to}",
            from = f.id.as_str(),
            pin = l.pin.as_str(),
            to = t.id.as_str()
        );
        self.set_status(ctx, msg);
        self.hot_link = None;
    }

    /// The wire under the pointer goes (the D key).
    pub fn remove_hot_wire(&mut self, ctx: &egui::Context, sc: &Scene, cx: &mut CanvasCx<'_>) -> bool {
        // the wire lit under the pointer, or the one under it when nothing was lit yet - never one
        // behind a node, nor one where the pointer is not over the canvas at all
        let mouse = self.last_mouse;
        let hot = self.hot_link;
        let li = match hot {
            Some(li) => Some(li),
            None if self.hovered && self.hit_node(sc, cx.view, mouse).is_none() => {
                self.hit_link(sc, cx.view, mouse)
            }
            None => None,
        };
        let Some(li) = li else {
            return false;
        };
        self.remove_wire(ctx, sc, cx, li);
        true
    }

    fn end_cut(&mut self, ctx: &egui::Context, sc: &Scene, cx: &mut CanvasCx<'_>, start: Pos2, end: Pos2) {
        if (end - start).length() < 3.0 {
            return;
        }
        let a = self.to_world(cx.view, start);
        let b = self.to_world(cx.view, end);
        let r = [a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])];
        let mut cut: Vec<Value> = Vec::new();
        for li in self.visible_links(sc, r) {
            let pts = self.link_points(sc, li);
            if !pts.windows(2).any(|w| crosses(a, b, w[0], w[1])) {
                continue;
            }
            let Some(l) = sc.links.get(li) else { continue };
            if let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) {
                cut.push(wire_value(&f.id, &l.pin, &t.id));
            }
        }
        if cut.is_empty() {
            self.set_status(ctx, "No wire crosses the cut");
            return;
        }
        cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
            ("op", Value::from("cut")),
            ("wires", Value::from(cut)),
        ]))));
    }

    fn end_link(&mut self, ctx: &egui::Context, sc: &Scene, cx: &mut CanvasCx<'_>, mouse: Pos2, grab: f32) {
        let Some(l) = self.linking.take() else { return };
        self.status = gz_ui::Tx::EMPTY;
        // a pin pressed and let go where it was is a click on its node: it is selected
        if (mouse - self.link_press).length() < 4.0 {
            let (Linking::From { node, .. } | Linking::Into { node }) = &l;
            if !cx.view.is_selected(node) {
                cx.view.select_only(node);
                self.clear_knots();
            }
            return;
        }
        match l {
            Linking::From { node: from, pin } => {
                let mut node = self.hit_node(sc, cx.view, mouse);
                match node {
                    Some(n) => {
                        // a refused drop says why
                        if let Some(t) = sc.nodes.get(n)
                            && let Err(why) = doc::can_link(cx.catalog, cx.graph, &from, &t.id)
                        {
                            if t.id != from {
                                // the graph's own English
                                self.set_status(ctx, gz_ui::Tx::ui(why));
                            }
                            return;
                        }
                    }
                    None => {
                        // a drop beside a box still means that box: its input sits on the edge
                        node = self.hit_node_near(sc, cx.view, mouse, sc.node_of(&from), grab);
                        if let Some(n) = node
                            && let Some(t) = sc.nodes.get(n)
                            && doc::can_link(cx.catalog, cx.graph, &from, &t.id).is_err()
                        {
                            node = None;
                        }
                    }
                }
                match node.and_then(|n| sc.nodes.get(n)) {
                    Some(t) => cx.ops.push(Op::Doc(DocOp::Connect {
                        from: from.to_string(),
                        pin: pin.to_string(),
                        to: t.id.to_string(),
                    })),
                    None if self.hovered => {
                        // dropped on empty space: a node made there is wired from the pin
                        let at = self.to_world(cx.view, mouse);
                        self.open_picker(ctx, cx, PickFor::FromPin { from, pin, at }, mouse);
                    }
                    None => {}
                }
            }
            Linking::Into { node: to } => {
                if let Some((n, k)) = self.hit_any_pin(sc, cx.view, mouse, grab)
                    && let Some(f) = sc.nodes.get(n)
                    && let Some(pin) = f.pins.get(k)
                {
                    match doc::can_link(cx.catalog, cx.graph, &f.id, &to) {
                        Ok(()) => cx.ops.push(Op::Doc(DocOp::Connect {
                            from: f.id.to_string(),
                            pin: pin.name.clone(),
                            to: to.to_string(),
                        })),
                        Err(why) => self.set_status(ctx, gz_ui::Tx::ui(why)),
                    }
                    return;
                }
                if let Some(n) = self.hit_node(sc, cx.view, mouse)
                    && let Some(f) = sc.nodes.get(n)
                {
                    if f.id == to {
                        return;
                    }
                    match f.pins.iter().find(|p| p.role == PinRole::Default) {
                        Some(pin) => match doc::can_link(cx.catalog, cx.graph, &f.id, &to) {
                            Ok(()) => cx.ops.push(Op::Doc(DocOp::Connect {
                                from: f.id.to_string(),
                                pin: pin.name.clone(),
                                to: to.to_string(),
                            })),
                            Err(why) => self.set_status(ctx, gz_ui::Tx::ui(why)),
                        },
                        None => self.set_status(
                            ctx,
                            gz_ui::tr!(
                                "{node} has no way out to wire from",
                                node = gz_ui::Tx::ui_in("kind", f.title.clone())
                            ),
                        ),
                    }
                    return;
                }
                if self.hovered {
                    let at = self.to_world(cx.view, mouse);
                    self.open_picker(ctx, cx, PickFor::IntoInput { to, at }, mouse);
                }
            }
        }
    }

    fn end_rewire(&mut self, ctx: &egui::Context, sc: &Scene, cx: &mut CanvasCx<'_>, mouse: Pos2, grab: f32) {
        let Some(r) = self.rewire.take() else { return };
        let (new_from, new_pin, new_to) = if r.target {
            let node = self
                .hit_node(sc, cx.view, mouse)
                .or_else(|| self.hit_node_near(sc, cx.view, mouse, sc.node_of(&r.from), grab));
            let Some(t) = node.and_then(|n| sc.nodes.get(n)) else { return };
            (r.from.clone(), r.pin.clone(), t.id.clone())
        } else {
            let Some((n, k)) = self.hit_any_pin(sc, cx.view, mouse, grab) else { return };
            let Some(f) = sc.nodes.get(n) else { return };
            let Some(pin) = f.pins.get(k) else { return };
            (f.id.clone(), SmolStr::new(&pin.name), r.to.clone())
        };
        if (new_from.as_str(), new_pin.as_str(), new_to.as_str())
            == (r.from.as_str(), r.pin.as_str(), r.to.as_str())
        {
            return;
        }
        if let Err(why) = doc::can_link(cx.catalog, cx.graph, &new_from, &new_to) {
            self.set_status(ctx, gz_ui::Tx::ui(why));
            return;
        }
        let ops = vec![
            op_table(&[
                ("op", Value::from("disconnect")),
                ("from", Value::from(r.from.as_str())),
                ("pin", Value::from(r.pin.as_str())),
                ("to", Value::from(r.to.as_str())),
            ]),
            op_table(&[
                ("op", Value::from("connect")),
                ("from", Value::from(new_from.as_str())),
                ("pin", Value::from(new_pin.as_str())),
                ("to", Value::from(new_to.as_str())),
            ]),
        ];
        cx.ops.push(Op::Doc(DocOp::ApplyOps(Value::from(ops))));
    }

    /// What the pointer is on, said on hover, in the window's language.
    fn hover_tip(&mut self, ctx: &egui::Context, sc: &Scene, cx: &CanvasCx<'_>, mouse: Pos2, hovered: bool) {
        let t = |en: &'static str| gz_ui::i18n::t(ctx, en);
        let busy = self.drag.is_some()
            || self.linking.is_some()
            || self.rewire.is_some()
            || self.marquee.is_some()
            || self.cut.is_some()
            || self.act_drag.as_ref().is_some_and(|a| a.dragging);
        if !hovered || !self.tip.is_empty() || busy {
            return;
        }
        if let Some(h) = self.hot_action {
            if h.more {
                self.tip = t("Show the rest of the actions in the Details panel");
            } else if let Some(sn) = sc.nodes.get(h.node)
                && let Some(row) = sn.action_at(h.row)
            {
                let slot = if row.exit {
                    gz_ui::tr!("On exit {n}", n = row.index + 1)
                } else {
                    gz_ui::tr!("On enter {n}", n = row.index + 1)
                };
                let mut tip =
                    format!("{}: {}", slot.text(ctx), gz_ui::Tx::ui_in("kind", row.title.as_str()).text(ctx));
                if !row.summary.is_empty() {
                    tip.push('\n');
                    // what it acts on: names and values, as they are
                    tip.push_str(&row.summary);
                }
                tip.push('\n');
                tip.push_str(&t("Double-click to edit it. Drag it within the node, onto another node, or just under a node to run it on exit"));
                self.tip = tip;
            }
            return;
        }
        if self.hot_knot.is_some() {
            self.tip = t("A reroute: click to select it, drag to move it, double-click to remove it");
            return;
        }
        if let Some(li) = self.hot_link
            && let Some(l) = sc.links.get(li)
            && let (Some(f), Some(to)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize))
        {
            let wire = if l.pin.as_str() == "next" {
                format!("{} -> {}", f.id, to.id)
            } else {
                format!("{} -> {} ({})", f.id, to.id, l.pin)
            };
            self.tip = format!(
                "{wire}\n{}",
                t("Double-click to add a reroute, Alt+click to remove, Alt+drag an end to rewire")
            );
        }
        let _ = (cx, mouse);
    }

    // ---- the keys (the tab hands them over while the canvas is the part worked in) ----------

    /// Whether the canvas takes keys now.
    #[must_use]
    pub fn takes_keys(&self, ctx: &egui::Context) -> bool {
        (self.hovered || self.focused) && !egui::Popup::is_any_open(ctx) && !ctx.egui_wants_keyboard_input()
    }

    /// Esc drops a gesture under way; `true` when one was. Every other key of the canvas is an
    /// action of the graph (`actions.rs`).
    pub fn escape(&mut self) -> bool {
        let busy = self.linking.is_some()
            || self.rewire.is_some()
            || self.marquee.is_some()
            || self.act_drag.is_some()
            || self.cut.is_some()
            || self.frame_drag.is_some()
            || self.drag.as_ref().is_some_and(|d| d.moved);
        self.linking = None;
        self.rewire = None;
        self.marquee = None;
        self.cut = None;
        self.alt_press = None;
        self.frame_drag = None;
        self.on_release = None;
        self.guides.clear();
        if self.drag.as_ref().is_some_and(|d| d.moved) {
            self.drag = None;
        }
        // drops the action back where it came from: the move only happens on release
        self.act_drag = None;
        busy
    }

    /// The selected nodes as a Lua fragment the clipboard carries.
    #[must_use]
    pub fn copy_text(graph: &Graph, selection: &[SmolStr]) -> Option<String> {
        if selection.is_empty() {
            return None;
        }
        let mut text_ = String::from("return {\n  nodes = {\n");
        for id in selection {
            let Some(n) = graph.find_node(id) else { continue };
            text_.push_str("    ");
            text_.push_str(&asset::write_node(n, 4));
            text_.push_str(",\n");
        }
        text_.push_str("  },\n}\n");
        Some(text_)
    }

    /// Pastes a fragment at the pointer (or the view's centre): fresh ids, the wires inside the
    /// fragment kept, kinds this document may not carry left out and named, one undo step.
    pub fn paste(&mut self, cx: &mut CanvasCx<'_>, clip: &str, at: Option<[f32; 2]>) {
        let anchor = at.unwrap_or_else(|| {
            if self.hovered { self.to_world(cx.view, self.last_mouse) } else { cx.view.center }
        });
        cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
            ("op", Value::from("paste")),
            ("text", Value::from(clip)),
            ("at", point(anchor)),
        ]))));
    }

    /// The world point under the pointer, or the view's centre when the pointer is elsewhere.
    #[must_use]
    pub fn pointer_world(&self, view: &ViewMirror) -> [f32; 2] {
        if self.hovered { self.to_world(view, self.last_mouse) } else { view.center }
    }

    /// The pointer's place on screen (the picker opens there), or the canvas's centre.
    #[must_use]
    pub fn pointer_screen(&self) -> Pos2 {
        if self.hovered { self.last_mouse } else { self.origin + self.size * 0.5 }
    }
}

/// Moves one action: within its slot, to the other slot or to another node - one undo step, and
/// the Details panel follows it to where it landed.
pub(super) fn move_action(
    cx: &mut CanvasCx<'_>,
    from: &str,
    from_exit: bool,
    index: usize,
    to: &str,
    to_exit: bool,
    at: usize,
) {
    // dropping an action on its own place is not a move
    if from == to && from_exit == to_exit && (at == index || at == index + 1) {
        return;
    }
    let mut pos = at;
    if from == to && from_exit == to_exit && pos > index {
        pos -= 1;
    }
    let dst_len =
        cx.graph.find_node(to).map_or(0, |n| if to_exit { n.on_exit.len() } else { n.on_enter.len() });
    let dst_len = if from == to && from_exit == to_exit { dst_len.saturating_sub(1) } else { dst_len };
    let pos = pos.min(dst_len);
    cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
        ("op", Value::from("move_action")),
        ("id", Value::from(from)),
        ("slot", Value::from(slot_name(from_exit))),
        ("index", Value::from((index + 1) as f64)),
        ("to_id", Value::from(to)),
        ("to_slot", Value::from(slot_name(to_exit))),
        ("to_index", Value::from((pos + 1) as f64)),
    ]))));
    cx.view.select_only(to);
    cx.view.slot = format!("{}:{pos}", slot_name(to_exit));
}
