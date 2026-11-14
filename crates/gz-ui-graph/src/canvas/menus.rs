//! The canvas's popups: the menus of a node, an action, a wire, a reroute, a comment frame and
//! the empty canvas, and the kind picker - the kit's `PickList`, filtered by what it is for (from
//! an output only kinds that take an input, into an input only kinds that go on, in an action
//! slot only actions), with Recent and Favourites on top. A menu row of the group Graph is its
//! action: the row shows the action's keys as bound now, and picking it runs the action.

use gz_nq::Kind;
use gz_ui::pick::{favourites, pick_popup, set_favourite};

use super::*;

/// The global favourites of the picker: one list in the editor's settings for every project.
const FAVOURITES: &str = "graph";

/// What a picker offers, by what the new node is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickWant {
    /// Anything a graph's canvas holds: triggers and main nodes.
    Any,
    /// A node a wire can enter (wired from an output): no trigger.
    TakesInput,
    /// A node a wire can leave (wired into an input): one with a way out.
    GoesOn,
    /// A node that goes on a wire: takes an input and goes on.
    OnWire,
    /// An action of an `on_enter` / `on_exit` list.
    Action,
    /// Another trigger (replacing one).
    Trigger,
}

/// The kinds a picker lists, in the document's vocabulary.
#[must_use]
pub fn pick_kinds(cat: &Catalog, behaviour: bool, want: PickWant) -> Vec<&Kind> {
    let mask = match want {
        PickWant::Any | PickWant::GoesOn => USE_TRIGGER | USE_MAIN,
        PickWant::TakesInput | PickWant::OnWire => USE_MAIN,
        PickWant::Action => USE_EXTRA,
        PickWant::Trigger => USE_TRIGGER,
    };
    let has_out = |k: &Kind| !k.pins.is_empty() || k.pins_from_cases;
    cat.kinds_for(mask, behaviour)
        .into_iter()
        .filter(|k| match want {
            PickWant::TakesInput => !cat.is_trigger(&k.id),
            PickWant::GoesOn => has_out(k),
            PickWant::OnWire => !cat.is_trigger(&k.id) && has_out(k),
            _ => true,
        })
        .collect()
}

impl PickFor {
    fn want(&self, cat: &Catalog, g: &Graph) -> PickWant {
        match self {
            PickFor::Canvas { .. } => PickWant::Any,
            PickFor::FromPin { .. } => PickWant::TakesInput,
            PickFor::IntoInput { .. } => PickWant::GoesOn,
            PickFor::Action { .. } => PickWant::Action,
            PickFor::OnWire { .. } => PickWant::OnWire,
            PickFor::Replace { node } => {
                if g.find_node(node).is_some_and(|n| cat.is_trigger(&n.kind)) {
                    PickWant::Trigger
                } else {
                    PickWant::TakesInput
                }
            }
        }
    }

    fn hint(&self) -> &'static str {
        match self {
            PickFor::Action { .. } => "Search actions",
            PickFor::Replace { .. } => "Replace with",
            _ => "Search nodes",
        }
    }
}

/// An action's menu row: its title (or `label`), icon and keys as bound now, off with its reason.
/// Picked, the tab runs it.
fn action_row(ui: &mut Ui, cx: &mut CanvasCx<'_>, id: &'static str, label: Option<&str>) -> bool {
    let Some(editor) = cx.editor else { return false };
    let Some(actions) = gz_app::Actions::of(editor) else { return false };
    let (a, keys) = {
        let t = actions.table.read();
        (t.get(id).copied(), t.keys_label(id))
    };
    let Some(a) = a else { return false };
    let (on, checked) = actions.with_cx(editor, |acx| ((a.enabled)(acx), a.checked.is_some_and(|f| f(acx))));
    let item = MenuItem {
        icon: a.icon.unwrap_or(Icon::None),
        keys: &keys,
        enabled: on.on(),
        selected: checked,
        tip: on.why(),
        ..MenuItem::default()
    };
    let picked = menu_item(ui, label.unwrap_or(a.title), item);
    if picked {
        cx.actions.push(id);
    }
    picked
}

impl Canvas {
    fn open_menu(&mut self, ctx: &egui::Context, m: Menu, at: Pos2) {
        self.menu = Some(m);
        self.menu_pos = at;
        egui::Popup::open_id(ctx, self.id.with("menu"));
    }

    /// A menu that asks for another one (Add Action...): it goes up after this one closed.
    fn chain_menu(&mut self, m: Menu, at: Pos2) {
        self.next_menu = Some((m, at));
    }

    /// The name of the popup that is up (`graph.view` answers it): node, wire, canvas, frame or
    /// picker.
    #[must_use]
    pub fn menu_up(&self, ctx: &egui::Context) -> Option<&'static str> {
        egui::Popup::is_id_open(ctx, self.id.with("menu"))
            .then(|| self.menu.as_ref().map(Menu::name))
            .flatten()
    }

    pub fn close_menu(&mut self, ctx: &egui::Context) {
        egui::Popup::close_id(ctx, self.id.with("menu"));
        self.menu = None;
    }

    /// Opens a picker: its rows are worked out once, here, and the list starts fresh.
    pub(super) fn open_picker(&mut self, ctx: &egui::Context, cx: &CanvasCx<'_>, for_: PickFor, at: Pos2) {
        self.open_picker_with(ctx, cx.catalog, cx.graph, cx.pick_memory, for_, at);
    }

    fn open_picker_with(
        &mut self,
        ctx: &egui::Context,
        cat: &Catalog,
        g: &Graph,
        memory: &str,
        for_: PickFor,
        at: Pos2,
    ) {
        self.pick_items = picker_items(ctx, cat, g, for_.want(cat, g));
        PickList::reset(ctx, ("gz-ui-graph.picker", self.id));
        sync_favourites(ctx, memory, true);
        let m = Menu::Picker(for_);
        self.open_menu(ctx, m, at);
    }

    /// The node picker at the pointer (Tab, Space): a node made there.
    pub fn open_picker_here(
        &mut self,
        ctx: &egui::Context,
        cat: &Catalog,
        g: &Graph,
        view: &ViewMirror,
        memory: &str,
    ) {
        let at = self.pointer_world(view);
        let screen = self.pointer_screen();
        self.open_picker_with(ctx, cat, g, memory, PickFor::Canvas { at }, screen);
    }

    /// The picker of another kind for a node (Replace Node).
    pub fn open_replace(
        &mut self,
        ctx: &egui::Context,
        cat: &Catalog,
        g: &Graph,
        view: &ViewMirror,
        memory: &str,
        node: &str,
    ) {
        let at = self.pointer_screen();
        let _ = view;
        self.open_picker_with(ctx, cat, g, memory, PickFor::Replace { node: SmolStr::new(node) }, at);
    }

    /// The picker as a wire dragged out of an output (`from`: node, pin) or into an input (`into`)
    /// and dropped a little way off puts it up: filtered by the pin, the new node wired to it.
    #[allow(clippy::too_many_arguments)]
    pub fn open_picker_wired(
        &mut self,
        ctx: &egui::Context,
        sc: &Scene,
        cat: &Catalog,
        g: &Graph,
        view: &ViewMirror,
        memory: &str,
        from: Option<(&str, &str)>,
        into: Option<&str>,
    ) -> Result<(), String> {
        let off = 160.0;
        let (for_, w) = match (from, into) {
            (Some((node, pin)), _) => {
                let i = sc.node_of(node).ok_or_else(|| format!("no node '{node}' in this graph"))?;
                let k = sc
                    .nodes
                    .get(i)
                    .and_then(|n| n.pin_index(pin))
                    .ok_or_else(|| format!("'{node}' has no output '{pin}'"))?;
                let p = self.output_pin_w(sc, i, k);
                let at = if sc.flow == Flow::Right { [p[0] + off, p[1]] } else { [p[0], p[1] + off] };
                (PickFor::FromPin { from: SmolStr::new(node), pin: SmolStr::new(pin), at }, at)
            }
            (None, Some(node)) => {
                let i = sc.node_of(node).ok_or_else(|| format!("no node '{node}' in this graph"))?;
                if sc.nodes.get(i).is_some_and(|n| n.trigger) {
                    return Err(format!("'{node}' is a trigger: it takes no wire in"));
                }
                let p = self.input_pin_w(sc, i);
                let at = if sc.flow == Flow::Right { [p[0] - off, p[1]] } else { [p[0], p[1] - off] };
                (PickFor::IntoInput { to: SmolStr::new(node), at }, at)
            }
            (None, None) => return Err("name the output (from_pin=node.pin) or the input (into=node)".into()),
        };
        let screen = self.to_screen(view, w);
        self.open_picker_with(ctx, cat, g, memory, for_, screen);
        Ok(())
    }

    /// What the picker that is up offers: what it is for and every kind on it.
    #[must_use]
    pub fn picker_info(&self, ctx: &egui::Context) -> Option<(&'static str, Vec<String>)> {
        if !egui::Popup::is_id_open(ctx, self.id.with("menu")) {
            return None;
        }
        let what = match self.menu.as_ref()? {
            Menu::Picker(PickFor::Canvas { .. }) | Menu::Canvas { .. } => "canvas",
            Menu::Picker(PickFor::FromPin { .. }) => "from_pin",
            Menu::Picker(PickFor::IntoInput { .. }) => "into_input",
            Menu::Picker(PickFor::Action { .. }) => "action",
            Menu::Picker(PickFor::OnWire { .. }) => "on_wire",
            Menu::Picker(PickFor::Replace { .. }) => "replace",
            _ => return None,
        };
        Some((what, self.pick_items.iter().map(|it| it.id.clone()).collect()))
    }

    /// The add-action picker of a node, put up at a screen point (the Details panel's Add Action).
    #[allow(clippy::too_many_arguments)]
    pub fn request_add_action(
        &mut self,
        ctx: &egui::Context,
        cat: &Catalog,
        g: &Graph,
        memory: &str,
        node: &str,
        exit: bool,
        at: Pos2,
    ) {
        self.open_picker_with(ctx, cat, g, memory, PickFor::Action { node: SmolStr::new(node), exit }, at);
    }

    /// Puts a menu up to look at (`graph.view menu=`): a node's (the selected one, or the first),
    /// a wire's (the first), the canvas's, a frame's or the picker.
    #[allow(clippy::too_many_arguments)]
    pub fn open_menu_named(
        &mut self,
        ctx: &egui::Context,
        sc: &Scene,
        cat: &Catalog,
        g: &Graph,
        view: &mut ViewMirror,
        memory: &str,
        name: &str,
    ) -> Result<(), String> {
        let center = self.origin + self.size * 0.5;
        match name {
            "none" => {
                self.close_menu(ctx);
                Ok(())
            }
            "node" => {
                let id = view.selection.first().cloned().or_else(|| sc.nodes.first().map(|n| n.id.clone()));
                let Some(id) = id else { return Err("the graph has no node".into()) };
                if !view.is_selected(&id) {
                    view.select_only(&id);
                }
                let at = sc.node_of(&id).map_or(center, |i| {
                    let r = self.rect_of(sc, view, i);
                    pos2(r.center().x, r.min.y + HEADER * Self::scale(view) * 0.5)
                });
                self.open_menu(ctx, Menu::Node(id), at);
                Ok(())
            }
            "wire" => {
                let Some((li, l)) = sc.links.iter().enumerate().next() else {
                    return Err("the graph has no wire".into());
                };
                let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) else {
                    return Err("the graph has no wire".into());
                };
                let pts = self.link_points(sc, li);
                let mid = wires::point_at(&pts, 0.5).map_or([0.0, 0.0], |(q, _)| q);
                let at = self.to_screen(view, mid);
                self.open_menu(
                    ctx,
                    Menu::Wire { from: f.id.clone(), pin: l.pin.clone(), to: t.id.clone(), at: mid },
                    at,
                );
                Ok(())
            }
            "frame" => {
                let Some(f) = sc.frames.first() else { return Err("the graph has no comment frame".into()) };
                let at = self.to_screen(view, [f.rect[0] + 24.0, f.rect[1] + FRAME_TITLE * 0.5]);
                self.open_menu(ctx, Menu::Frame(f.index), at);
                Ok(())
            }
            "canvas" => {
                let at = self.to_world(view, center);
                self.pick_items = picker_items(ctx, cat, g, PickWant::Any);
                PickList::reset(ctx, ("gz-ui-graph.picker", self.id));
                sync_favourites(ctx, memory, true);
                self.open_menu(ctx, Menu::Canvas { at }, center);
                Ok(())
            }
            "picker" => {
                let at = self.to_world(view, center);
                self.open_picker_with(ctx, cat, g, memory, PickFor::Canvas { at }, center);
                Ok(())
            }
            other => Err(format!("menu is node, wire, canvas, frame, picker or none, not '{other}'")),
        }
    }

    pub(super) fn open_context_menu(
        &mut self,
        ctx: &egui::Context,
        sc: &Scene,
        cx: &mut CanvasCx<'_>,
        mouse: Pos2,
        ppp: f32,
    ) {
        if let Some((link, index)) = self.hit_knot(sc, cx.view, mouse, (5.0 * Self::scale(cx.view)).max(6.0))
        {
            self.open_menu(ctx, Menu::Knot { link, index }, mouse);
            return;
        }
        if let Some(i) = self.hit_node(sc, cx.view, mouse) {
            let Some(n) = sc.nodes.get(i) else { return };
            if !cx.view.is_selected(&n.id) {
                cx.view.select_only(&n.id);
            }
            let detail = Self::detail(cx.view, ppp, n.size[0]);
            if let Some(hit) = self.hit_action(sc, cx.view, i, mouse, detail).filter(|h| !h.more)
                && let Some(row) = n.action_at(hit.row)
            {
                self.act_drag = None;
                cx.view.slot = format!("{}:{}", slot_name(row.exit), row.index);
                self.open_menu(
                    ctx,
                    Menu::Action { node: n.id.clone(), exit: row.exit, index: row.index },
                    mouse,
                );
            } else {
                self.open_menu(ctx, Menu::Node(n.id.clone()), mouse);
            }
            return;
        }
        if let Some(li) = self.hit_link(sc, cx.view, mouse)
            && let Some(l) = sc.links.get(li)
        {
            let (Some(f), Some(t)) = (sc.nodes.get(l.from as usize), sc.nodes.get(l.to as usize)) else {
                return;
            };
            let at = self.to_world(cx.view, mouse);
            self.open_menu(
                ctx,
                Menu::Wire { from: f.id.clone(), pin: l.pin.clone(), to: t.id.clone(), at },
                mouse,
            );
            return;
        }
        if let Some((fi, _)) = self.hit_frame(sc, cx.view, mouse) {
            self.open_menu(ctx, Menu::Frame(fi), mouse);
            return;
        }
        // the empty canvas: its few actions over the picker
        let at = self.to_world(cx.view, mouse);
        self.pick_items = picker_items(ctx, cx.catalog, cx.graph, PickWant::Any);
        PickList::reset(ctx, ("gz-ui-graph.picker", self.id));
        sync_favourites(ctx, cx.pick_memory, true);
        self.open_menu(ctx, Menu::Canvas { at }, mouse);
    }

    /// A frame's title, edited where it is drawn.
    pub(super) fn draw_frame_title_edit(&mut self, ui: &mut Ui, sc: &Scene, cx: &mut CanvasCx<'_>) {
        let Some((index, mut title, focus)) = self.frame_title.take() else { return };
        let Some(f) = sc.frames.iter().find(|f| f.index == index) else { return };
        let s = Self::scale(cx.view);
        let a = self.to_screen(cx.view, [f.rect[0], f.rect[1]]);
        let w = ((f.rect[2] - f.rect[0]) * s - 12.0).clamp(120.0, 480.0);
        let r = Rect::from_min_size(pos2(a.x + 6.0, a.y + 4.0), vec2(w, Font::Body.line() + 8.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r));
        let resp = gz_ui::grid::text_field(&mut child, &mut title);
        if focus {
            resp.request_focus();
        }
        let (enter, esc) = ui.ctx().input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        if esc {
            return;
        }
        if enter || resp.lost_focus() {
            if title.trim() != f.title {
                cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
                    ("op", Value::from("frame")),
                    ("action", Value::from("set")),
                    ("index", Value::from((index + 1) as f64)),
                    ("title", Value::from(title.trim())),
                ]))));
            }
            return;
        }
        self.frame_title = Some((index, title, false));
    }

    /// What a pick does: a node made and wired the way the picker was asked for, an action added,
    /// a kind replaced.
    fn apply_pick(&mut self, cx: &mut CanvasCx<'_>, for_: PickFor, kind: &str) {
        let cat = cx.catalog;
        let g = cx.graph;
        let last = kind.rsplit('.').next().unwrap_or(kind);
        let base = if last == "start" || last == "when" { "trigger" } else { last };
        let id = g.free_node_id(base);
        let probe = Node::new(&id, kind);
        let size = layout::node_size(&layout::BoxCx::of(cat, g), &probe);
        let flow = g.flow();
        let add = |at: [f32; 2]| {
            op_table(&[
                ("op", Value::from("add")),
                ("id", Value::from(id.as_str())),
                ("kind", Value::from(kind)),
                ("at", point([layout::snap(at[0]), layout::snap(at[1])])),
            ])
        };
        let ops: Vec<Value> = match &for_ {
            PickFor::Canvas { at } => vec![add(*at)],
            PickFor::FromPin { from, pin, at } => {
                // the new node's input lands where the wire was dropped
                let pos = match flow {
                    Flow::Right => [at[0], at[1] - HEADER * 0.5],
                    Flow::Down => [at[0] - size[0] * 0.5, at[1]],
                };
                vec![
                    add(pos),
                    op_table(&[
                        ("op", Value::from("connect")),
                        ("from", Value::from(from.as_str())),
                        ("pin", Value::from(pin.as_str())),
                        ("to", Value::from(id.as_str())),
                    ]),
                ]
            }
            PickFor::IntoInput { to, at } => {
                let (pins, _) = layout::pins(cat, &probe);
                let Some(out) = layout::default_pin(&pins).map(str::to_owned) else { return };
                let pos = match flow {
                    Flow::Right => [at[0] - size[0], at[1] - HEADER * 0.5],
                    Flow::Down => [at[0] - size[0] * 0.5, at[1] - size[1]],
                };
                vec![
                    add(pos),
                    op_table(&[
                        ("op", Value::from("connect")),
                        ("from", Value::from(id.as_str())),
                        ("pin", Value::from(out.as_str())),
                        ("to", Value::from(to.as_str())),
                    ]),
                ]
            }
            PickFor::OnWire { from, pin, to, at } => {
                let pos = [at[0] - size[0] * 0.5, at[1] - size[1] * 0.5];
                vec![op_table(&[
                    ("op", Value::from("insert_on_wire")),
                    ("from", Value::from(from.as_str())),
                    ("pin", Value::from(pin.as_str())),
                    ("to", Value::from(to.as_str())),
                    ("kind", Value::from(kind)),
                    ("id", Value::from(id.as_str())),
                    ("at", point(pos)),
                ])]
            }
            PickFor::Replace { node } => {
                cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
                    ("op", Value::from("replace")),
                    ("node", Value::from(node.as_str())),
                    ("kind", Value::from(kind)),
                ]))));
                return;
            }
            PickFor::Action { node, exit } => {
                let mut a = Value::table();
                a.set("kind", kind);
                let len =
                    g.find_node(node).map_or(0, |n| if *exit { n.on_exit.len() } else { n.on_enter.len() });
                cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
                    ("op", Value::from("add_action")),
                    ("id", Value::from(node.as_str())),
                    ("slot", Value::from(slot_name(*exit))),
                    ("action", a),
                ]))));
                cx.view.select_only(node);
                cx.view.slot = format!("{}:{len}", slot_name(*exit));
                self.want_focus_action = true;
                return;
            }
        };
        cx.ops.push(Op::Doc(DocOp::ApplyOps(Value::from(ops))));
        cx.view.select_only(&id);
    }

    #[allow(clippy::too_many_lines)]
    pub(super) fn draw_menus(&mut self, ui: &mut Ui, sc: &Scene, cx: &mut CanvasCx<'_>) {
        let ctx = ui.ctx().clone();
        let popup_id = self.id.with("menu");
        if !egui::Popup::is_id_open(&ctx, popup_id) {
            self.menu = None;
            if let Some((m, at)) = self.next_menu.take() {
                match m {
                    Menu::Picker(for_) => self.open_picker(&ctx, cx, for_, at),
                    m => self.open_menu(&ctx, m, at),
                }
            }
            return;
        }
        let Some(menu) = self.menu.clone() else {
            egui::Popup::close_id(&ctx, popup_id);
            return;
        };
        let anchor = Rect::from_min_size(self.menu_pos, Vec2::ZERO);
        let mut close = false;
        let picker_salt = ("gz-ui-graph.picker", self.id);
        if let Menu::Picker(for_) = &menu {
            let items = std::mem::take(&mut self.pick_items);
            let list = PickList::new(picker_salt, &items)
                .remember(cx.pick_memory)
                .hint(for_.hint())
                .width(gz_ui::units::em(24.0))
                .max_height(420.0);
            let picked = pick_popup(&ctx, popup_id, anchor, ui.layer_id(), list);
            sync_favourites(&ctx, cx.pick_memory, false);
            if let Some(kind) = picked.and_then(|i| items.get(i)).map(|it| it.id.clone()) {
                self.apply_pick(cx, for_.clone(), &kind);
            }
            self.pick_items = items;
            return;
        }
        let items = std::mem::take(&mut self.pick_items);
        let at_screen = self.menu_pos;
        menu_popup(&ctx, popup_id, anchor, ui.layer_id(), |ui| match &menu {
            Menu::Node(id) => {
                let trigger = cx.graph.find_node(id).is_some_and(|n| cx.catalog.is_trigger(&n.kind));
                action_row(ui, cx, "graph.rename", None);
                if menu_item(ui, "Replace Node...", MenuItem::default()) {
                    self.chain_menu(Menu::Picker(PickFor::Replace { node: id.clone() }), at_screen);
                }
                if menu_item(ui, "Duplicate", MenuItem { icon: Icon::Duplicate, ..MenuItem::default() }) {
                    cx.ops.push(Op::Doc(DocOp::Canvas(CanvasOp::Duplicate { wires: false })));
                }
                action_row(ui, cx, "graph.duplicate_with_wires", None);
                if menu_item(ui, "Copy", MenuItem { icon: Icon::Copy, keys: "Ctrl+C", ..MenuItem::default() })
                    && let Some(t) = Self::copy_text(cx.graph, &cx.view.selection)
                {
                    ctx.copy_text(t);
                }
                ui.separator();
                if menu_item(
                    ui,
                    "Add Action on Enter...",
                    MenuItem { enabled: !trigger, ..MenuItem::default() },
                ) {
                    self.chain_menu(
                        Menu::Picker(PickFor::Action { node: id.clone(), exit: false }),
                        at_screen,
                    );
                }
                if menu_item(
                    ui,
                    "Add Action on Exit...",
                    MenuItem { enabled: !trigger, ..MenuItem::default() },
                ) {
                    self.chain_menu(
                        Menu::Picker(PickFor::Action { node: id.clone(), exit: true }),
                        at_screen,
                    );
                }
                let disabled = cx.graph.find_node(id).is_some_and(|n| n.disabled);
                let collapsed = cx.graph.find_node(id).is_some_and(|n| n.collapsed);
                action_row(ui, cx, "graph.disable", Some(if disabled { "Enable" } else { "Disable" }));
                action_row(ui, cx, "graph.collapse", Some(if collapsed { "Expand" } else { "Collapse" }));
                action_row(ui, cx, "graph.bookmark", None);
                ui.separator();
                gz_ui::menu::submenu(ui, "Select", Icon::None, true, |ui| {
                    action_row(ui, cx, "graph.select_connected", None);
                    action_row(ui, cx, "graph.select_upstream", None);
                    action_row(ui, cx, "graph.select_downstream", None);
                });
                gz_ui::menu::submenu(ui, "Arrange", Icon::AutoLayout, true, |ui| {
                    for id in [
                        "graph.align_left",
                        "graph.align_centre",
                        "graph.align_right",
                        "graph.align_top",
                        "graph.align_middle",
                        "graph.align_bottom",
                    ] {
                        action_row(ui, cx, id, None);
                    }
                    ui.separator();
                    action_row(ui, cx, "graph.distribute_horizontally", None);
                    action_row(ui, cx, "graph.distribute_vertically", None);
                    action_row(ui, cx, "graph.straighten", None);
                    ui.separator();
                    action_row(ui, cx, "graph.auto_layout_selection", None);
                    action_row(ui, cx, "graph.auto_layout_branch", None);
                });
                action_row(ui, cx, "graph.add_frame", None);
                action_row(ui, cx, "graph.frame_selection", None);
                action_row(ui, cx, "graph.connect_nearest", None);
                action_row(ui, cx, "graph.bypass", None);
                if menu_item(ui, "Disconnect Outputs", MenuItem::default()) {
                    cx.ops.push(Op::Doc(DocOp::Disconnect {
                        from: id.to_string(),
                        pin: String::new(),
                        to: String::new(),
                    }));
                }
                ui.separator();
                if menu_item(
                    ui,
                    "Delete",
                    MenuItem { icon: Icon::Delete, keys: "Del", danger: true, ..MenuItem::default() },
                ) && !cx.view.selection.is_empty()
                {
                    cx.ops.push(Op::Doc(DocOp::Canvas(CanvasOp::Delete)));
                }
            }
            Menu::Action { node, exit, index } => {
                let gn = cx.graph.find_node(node);
                let list_len = gn.map_or(0, |n| if *exit { n.on_exit.len() } else { n.on_enter.len() });
                let other_len = gn.map_or(0, |n| if *exit { n.on_enter.len() } else { n.on_exit.len() });
                let valid = *index < list_len;
                let enabled = |on: bool| MenuItem { enabled: on, ..MenuItem::default() };
                if menu_item(ui, "Duplicate Action", enabled(valid))
                    && let Some(a) =
                        gn.and_then(|n| (if *exit { &n.on_exit } else { &n.on_enter }).get(*index))
                {
                    cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
                        ("op", Value::from("add_action")),
                        ("id", Value::from(node.as_str())),
                        ("slot", Value::from(slot_name(*exit))),
                        ("index", Value::from((*index + 2) as f64)),
                        ("action", asset::action_to_value(a)),
                    ]))));
                }
                if menu_item(ui, "Move Up", enabled(valid && *index > 0)) {
                    input::move_action(cx, node, *exit, *index, node, *exit, index - 1);
                }
                if menu_item(ui, "Move Down", enabled(valid && index + 1 < list_len)) {
                    input::move_action(cx, node, *exit, *index, node, *exit, index + 2);
                }
                if menu_item(ui, if *exit { "Move to On Enter" } else { "Move to On Exit" }, enabled(valid)) {
                    input::move_action(cx, node, *exit, *index, node, !*exit, other_len);
                }
                ui.separator();
                if menu_item(
                    ui,
                    "Remove Action",
                    MenuItem { enabled: valid, danger: true, ..MenuItem::default() },
                ) {
                    cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
                        ("op", Value::from("remove_action")),
                        ("id", Value::from(node.as_str())),
                        ("slot", Value::from(slot_name(*exit))),
                        ("index", Value::from((*index + 1) as f64)),
                    ]))));
                    cx.view.slot.clear();
                }
            }
            Menu::Wire { from, pin, to, at } => {
                gz_ui::text::label(
                    ui,
                    &format!("{from}.{pin} -> {to}"),
                    Font::Small,
                    palette(&ctx).text_muted,
                    gz_ui::Clip::Middle,
                );
                if menu_item(ui, "Insert Node...", MenuItem::default()) {
                    self.chain_menu(
                        Menu::Picker(PickFor::OnWire {
                            from: from.clone(),
                            pin: pin.clone(),
                            to: to.clone(),
                            at: *at,
                        }),
                        at_screen,
                    );
                }
                let li = sc.links.iter().position(|l| {
                    l.pin == *pin
                        && sc.nodes.get(l.from as usize).is_some_and(|n| n.id == *from)
                        && sc.nodes.get(l.to as usize).is_some_and(|n| n.id == *to)
                });
                if menu_item(ui, "Add Reroute Here", MenuItem::default())
                    && let Some(li) = li
                    && let Some(l) = sc.links.get(li)
                {
                    let t = input::arc_of(&l.points, *at);
                    let k = l.via.iter().filter(|v| input::arc_of(&l.points, **v) < t).count();
                    let mut via = l.via.clone();
                    via.insert(k, *at);
                    self.push_route(sc, cx, li, &via);
                }
                let has_via = li.and_then(|i| sc.links.get(i)).is_some_and(|l| !l.via.is_empty());
                if menu_item(ui, "Remove Reroutes", MenuItem { enabled: has_via, ..MenuItem::default() })
                    && let Some(li) = li
                {
                    self.push_route(sc, cx, li, &[]);
                }
                ui.separator();
                if menu_item(
                    ui,
                    "Remove Wire",
                    MenuItem { icon: Icon::Delete, danger: true, ..MenuItem::default() },
                ) {
                    cx.ops.push(Op::Doc(DocOp::Disconnect {
                        from: from.to_string(),
                        pin: pin.to_string(),
                        to: to.to_string(),
                    }));
                }
            }
            Menu::Knot { link, index } => {
                if menu_item(ui, "Remove Reroute", MenuItem { icon: Icon::Delete, ..MenuItem::default() })
                    && let Some(l) = sc.links.get(*link)
                {
                    let mut via = l.via.clone();
                    if *index < via.len() {
                        via.remove(*index);
                    }
                    self.push_route(sc, cx, *link, &via);
                }
                if menu_item(ui, "Remove Every Reroute of the Wire", MenuItem::default()) {
                    self.push_route(sc, cx, *link, &[]);
                }
            }
            Menu::Frame(index) => {
                let title =
                    sc.frames.iter().find(|f| f.index == *index).map(|f| f.title.clone()).unwrap_or_default();
                if menu_item(ui, "Rename Frame", MenuItem { icon: Icon::Rename, ..MenuItem::default() }) {
                    self.frame_title = Some((*index, title, true));
                }
                gz_ui::text::label(ui, "Tone", Font::Small, palette(&ctx).text_muted, gz_ui::Clip::End);
                ui.horizontal(|ui| {
                    let p = palette(&ctx);
                    for tone in 0..10u8 {
                        let (r, resp) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::click());
                        ui.painter().rect_filled(
                            r.shrink(2.0),
                            CornerRadius::same(3),
                            p.family(usize::from(tone)),
                        );
                        if resp.hovered() {
                            ui.painter().rect_stroke(
                                r,
                                CornerRadius::same(3),
                                Stroke::new(1.0, p.text),
                                StrokeKind::Inside,
                            );
                        }
                        if resp.clicked() {
                            cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
                                ("op", Value::from("frame")),
                                ("action", Value::from("set")),
                                ("index", Value::from((*index + 1) as f64)),
                                ("tone", Value::from(f64::from(tone))),
                            ]))));
                            close = true;
                        }
                    }
                });
                if menu_item(ui, "Select What It Holds", MenuItem::default())
                    && let Some(f) = sc.frames.iter().find(|f| f.index == *index)
                {
                    let r = f.rect;
                    cx.view.selection = sc
                        .nodes
                        .iter()
                        .filter(|n| {
                            let b = n.rect();
                            b[0] >= r[0] && b[1] >= r[1] && b[2] <= r[2] && b[3] <= r[3]
                        })
                        .map(|n| n.id.clone())
                        .collect();
                    cx.view.slot.clear();
                }
                ui.separator();
                if menu_item(
                    ui,
                    "Remove Frame",
                    MenuItem { icon: Icon::Delete, danger: true, ..MenuItem::default() },
                ) {
                    cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
                        ("op", Value::from("frame")),
                        ("action", Value::from("remove")),
                        ("index", Value::from((*index + 1) as f64)),
                    ]))));
                }
            }
            Menu::Canvas { at } => {
                if menu_item(
                    ui,
                    "Paste",
                    MenuItem { icon: Icon::Paste, keys: "Ctrl+V", ..MenuItem::default() },
                ) {
                    self.want_paste = Some(*at);
                }
                if menu_item(ui, "Add Comment Frame", MenuItem { icon: Icon::Group, ..MenuItem::default() }) {
                    cx.ops.push(Op::Doc(DocOp::Apply(op_table(&[
                        ("op", Value::from("frame")),
                        ("action", Value::from("add")),
                        ("nodes", Value::from(Vec::<Value>::new())),
                        ("x", Value::from(f64::from(layout::snap(at[0])))),
                        ("y", Value::from(f64::from(layout::snap(at[1])))),
                    ]))));
                }
                action_row(ui, cx, "graph.frame_all", None);
                action_row(ui, cx, "graph.auto_layout", None);
                ui.separator();
                let list = PickList::new(picker_salt, &items)
                    .remember(cx.pick_memory)
                    .hint("Add a node")
                    .width(gz_ui::units::em(24.0))
                    .max_height(360.0);
                let picked = list.show(ui);
                if let Some(kind) = picked.item.and_then(|i| items.get(i)).map(|it| it.id.clone()) {
                    self.apply_pick(cx, PickFor::Canvas { at: *at }, &kind);
                    close = true;
                }
                if picked.close {
                    close = true;
                }
            }
            Menu::Picker(_) => {}
        });
        self.pick_items = items;
        if matches!(menu, Menu::Canvas { .. }) {
            sync_favourites(&ctx, cx.pick_memory, false);
        }
        if close {
            egui::Popup::close_id(&ctx, popup_id);
        }
    }
}

/// The picker's rows of a want, grouped by family and then by title, each with its icon in its
/// family's colour and what it does as its second line.
fn picker_items(ctx: &egui::Context, cat: &Catalog, g: &Graph, want: PickWant) -> Vec<PickItem> {
    let p = palette(ctx);
    let behaviour = g.is_behaviour();
    let mut rows: Vec<(usize, PickItem)> = pick_kinds(cat, behaviour, want)
        .into_iter()
        .map(|k| {
            let icon = look::kind_icon(cat, &k.id);
            let family = look::family_of(icon);
            let first = look::first_sentence(&look::kind_desc(cat, &k.id, behaviour));
            let item = PickItem::new(k.id.clone(), look::kind_title(cat, &k.id, behaviour))
                .icon(icon)
                .tint(look::family_color(&p, family, 1.0))
                .detail(if first.is_empty() { k.id.clone() } else { first });
            (family.order(), item)
        })
        .collect();
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.label.cmp(&b.1.label)));
    rows.into_iter().map(|(_, it)| it).collect()
}

/// The favourites are one list of the editor's settings for every project, while the picker
/// keeps them under the project's memory key with its Recent: they are copied in when a picker
/// opens and back out when a star changes them.
fn sync_favourites(ctx: &egui::Context, memory: &str, inward: bool) {
    if memory == FAVOURITES {
        return;
    }
    let (global, local) = (favourites(ctx, FAVOURITES), favourites(ctx, memory));
    if global == local {
        return;
    }
    let (from, to, into) = if inward { (&global, &local, memory) } else { (&local, &global, FAVOURITES) };
    for id in to.iter().filter(|id| !from.contains(id)) {
        set_favourite(ctx, into, id, false);
    }
    for id in from.iter().filter(|id| !to.contains(id)) {
        set_favourite(ctx, into, id, true);
    }
}
