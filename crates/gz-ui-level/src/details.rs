//! Details (role `selection`) and Level Settings (role `world`): the header, "Search N
//! properties" with Expand / Collapse All, the kit's property grid over `gz-scene`'s rows and a
//! help footer (`UILPropertiesFrom.cpp`, `UIWorldPropertiesFrom.cpp`, `UIPropertiesForm.cpp`).
//!
//! The rows are prepared off the UI thread ([`prepare`], a `Query` keyed by the scene's
//! revision, the library's and the search). An edit is a `SceneCmd`: a transform row sends
//! merged undo steps while it is dragged and seals the gesture when the pointer lets go; any
//! other value is sent once, when the drag or the typing ends. Choose rows are pick fields that
//! open `###dlg.choose`; a multi-line text is its first line and Edit, an M dialog
//! `###dlg.edit_text`.

use std::sync::Arc;

use egui::{Color32, Id, Ui, Vec2};
use gz_base::SmolStr;
use gz_editor::Editor;
use gz_entities::PropContext;
use gz_query::Query;
use gz_scene::props::{self, Effect, Kind as RowKind, MergedRow, PropCx, Val};
use gz_scene::{ObjClass, SceneCmd, SceneService, SceneState};
use gz_ui::grid::{self, PickAction, PropFlags, PropertyGrid};
use gz_ui::text::Clip;
use gz_ui::{Dialog, DialogSize, Font, Icon, Kind, SearchField, Tone, icon_button, state};
use rustc_hash::FxHashMap;

use crate::choose::{Chooser, Purpose};
use crate::lists::ListKey;
use crate::local::local_panel_header;

/// Details' rows (0) or Level Settings' (1), as `property_inspector` names them.
pub const SELECTION: usize = 0;
pub const WORLD: usize = 1;

/// A level of the grid: its own rows, then its categories, each in the order met.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Row(MergedRow),
    Section(String, Vec<Node>),
}

/// What a panel draws, prepared off the UI thread.
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    pub nodes: Vec<Node>,
    pub total: usize,
    pub visible: usize,
    pub icon: Icon,
    pub name: String,
    pub kind: String,
}

/// `rows_of` of `gz-scene`'s `property_inspector`: the merged rows of the selected objects of
/// the target class, or the scene's own.
#[must_use]
pub fn rows(
    state: &SceneState,
    lib: &gz_scene::library::Library,
    target: usize,
    ctx: &PropContext,
) -> Vec<MergedRow> {
    gz_scene::selection_rows(state, lib, target == WORLD, ctx)
}

fn insert(nodes: &mut Vec<Node>, path: &[&str], row: MergedRow) {
    match path.split_first() {
        None => nodes.push(Node::Row(row)),
        Some((head, rest)) => {
            let at = nodes.iter().position(|n| matches!(n, Node::Section(t, _) if t == head));
            let i = match at {
                Some(i) => i,
                None => {
                    nodes.push(Node::Section((*head).to_owned(), Vec::new()));
                    nodes.len() - 1
                }
            };
            if let Some(Node::Section(_, children)) = nodes.get_mut(i) {
                insert(children, rest, row);
            }
        }
    }
}

/// A level's rows first, then its categories.
fn order(nodes: &mut Vec<Node>) {
    nodes.sort_by_key(|n| matches!(n, Node::Section(..)));
    for n in nodes {
        if let Node::Section(_, c) = n {
            order(c);
        }
    }
}

/// The rows as a tree of categories, filtered by `filter` the way `property_inspector`
/// counts (`visible`), with the header.
#[must_use]
pub fn prepare(
    state: &SceneState,
    lib: &gz_scene::library::Library,
    target: usize,
    filter: &str,
    ctx: &PropContext,
) -> Prepared {
    let all = rows(state, lib, target, ctx);
    let f = filter.to_lowercase();
    let mut nodes = Vec::new();
    let mut visible = 0;
    for r in &all {
        if !f.is_empty() && !r.row.key.to_lowercase().contains(&f) && !r.row.label.to_lowercase().contains(&f)
        {
            continue;
        }
        visible += 1;
        let parts: Vec<&str> = r.row.path.split('\\').collect();
        let dirs = parts.split_last().map_or(&[][..], |(_, d)| d);
        insert(&mut nodes, dirs, r.clone());
    }
    order(&mut nodes);
    let (icon, name, kind) = if target == WORLD {
        let name = state
            .meta
            .file
            .as_ref()
            .and_then(|f| f.file_name())
            .map_or_else(|| "Untitled".to_owned(), |n| n.to_string_lossy().into_owned());
        (Icon::Level, name, "Level".to_owned())
    } else {
        header(state)
    };
    Prepared { nodes, total: all.len(), visible, icon, name, kind }
}

/// `RefreshHeader`: one object names itself, several say how many and of what kind.
fn header(state: &SceneState) -> (Icon, String, String) {
    let class = state.tools.placing;
    let selected: Vec<_> = state.selected_in_order().iter().filter_map(|id| state.get(*id)).collect();
    match selected.as_slice() {
        [] => (crate::classes::icon(class), "Nothing Selected".to_owned(), String::new()),
        [one] => (crate::classes::icon(one.class), one.name.to_string(), one.class.desc().to_owned()),
        many => {
            let first = many.first().map_or(class, |o| o.class);
            let uniform = many.iter().all(|o| o.class == first);
            let icon = if uniform { crate::classes::icon(first) } else { Icon::Group };
            (
                icon,
                format!("{} objects", many.len()),
                if uniform { first.desc().to_owned() } else { "Several kinds".to_owned() },
            )
        }
    }
}

/// A multi-line value: shown as its first line with Edit.
fn multi_line(key: &str, text: &str) -> bool {
    text.contains('\n') || key.ends_with("custom_data")
}

fn is_transform(key: &str) -> bool {
    matches!(key, "pos" | "rot" | "scale")
}

fn argb_of(c: Color32) -> u32 {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    (u32::from(a) << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

fn color_of(argb: u32) -> Color32 {
    Color32::from_rgba_unmultiplied((argb >> 16) as u8, (argb >> 8) as u8, argb as u8, (argb >> 24) as u8)
}

fn fcolor_argb(c: [f32; 4]) -> u32 {
    let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    (b(c[3]) << 24) | (b(c[0]) << 16) | (b(c[1]) << 8) | b(c[2])
}

fn vec_text(v: [f32; 3]) -> String {
    format!("{}, {}, {}", v[0], v[1], v[2])
}

/// An edit to send: the row, its value as `property_set` takes text, and whether it is a live
/// step of a drag (merged into one undo step).
#[derive(Clone, Debug, PartialEq)]
struct Commit {
    key: String,
    text: String,
    live: bool,
}

/// Applies a row's value as text, the way `property_set` does, as a `SceneCmd`; a transform
/// row's live steps merge under `merge`.
pub async fn set_value(
    editor: &Arc<Editor>,
    target: usize,
    key: &str,
    text: &str,
    merge: Option<SmolStr>,
) -> Result<(), String> {
    let svc = SceneService::of(editor).map_err(|e| e.message)?;
    let snap = svc.store.snapshot();
    let all = rows(&snap, &svc.library, target, &svc.prop_cx());
    let row = props::find(&all, key).ok_or_else(|| format!("no row '{key}'"))?.clone();
    let k = row.row.key.clone();
    let parsed = props::parse(&row.row, text).map_err(|e| format!("{} {e}", row.row.label))?;
    let origin = editor.ui_origin();
    if target == WORLD {
        let (level, tools) =
            props::set_world(&snap, &k, &parsed).map_err(|e| format!("{} {e}", row.row.label))?;
        svc.store
            .send(
                SceneCmd::SetWorld {
                    level: Box::new(level),
                    tools,
                    serial: snap.serial,
                    label: "Level Settings".into(),
                },
                origin,
            )
            .await
            .map_err(|e| e.message)?;
        return Ok(());
    }
    let ctx = svc.prop_cx();
    let cx = PropCx { lib: Some(&svc.library), entity: &ctx };
    let mut objects = Vec::new();
    let mut transforms = Vec::new();
    for obj in snap.selected_in_order().iter().filter_map(|id| snap.get(*id)) {
        match props::set_object(&snap, obj, &k, &parsed, &cx).map_err(|e| format!("{} {e}", row.row.label))? {
            Effect::Object(n) => objects.push(n),
            Effect::Objects(list) => objects.extend(list),
            Effect::Transform(e) => transforms.push((obj.id, e)),
            Effect::Nothing => {}
        }
    }
    if !transforms.is_empty() {
        svc.store
            .send(
                SceneCmd::Transform { items: transforms, members: true, label: "Transform".into(), merge },
                origin,
            )
            .await
            .map_err(|e| e.message)?;
    }
    if !objects.is_empty() {
        svc.store
            .send(SceneCmd::ReplaceObjects { objects, serial: snap.serial, label: "Property".into() }, origin)
            .await
            .map_err(|e| e.message)?;
    }
    Ok(())
}

/// Sends an edit; a refusal comes back as a toast.
fn send(editor: &Arc<Editor>, target: usize, c: Commit) {
    // the motion's buttons that need the frame clock, a file or a sheet
    if target == SELECTION && crate::motion_ui::press(editor, &c.key, &c.text) {
        return;
    }
    let ed = editor.clone();
    let merge = c.live.then(|| SmolStr::new(format!("details:{}", c.key)));
    editor.exec.spawn(async move {
        if let Err(e) = set_value(&ed, target, &c.key, &c.text, merge).await
            && let Ok(q) = gz_app::UiQueue::of(&ed)
        {
            q.post(move |_, ctx| gz_ui::state::toast(ctx, Tone::Error, &e));
        }
    });
}

/// The text dialog of a row.
#[derive(Clone, Debug)]
struct EditText {
    key: String,
    label: String,
    text: String,
    /// The objects the row belonged to when it opened: a selection that moved on leaves it.
    serial_target: Vec<gz_scene::ObjId>,
}

/// One of the two panels.
pub struct PropsPanel {
    pub target: usize,
    query: Query<(usize, String), Prepared>,
    search: String,
    /// The filter as last read from the scene's view (`property_inspector filter=` moves it).
    synced: String,
    /// Grid sections fold anew when this moves (Expand / Collapse All).
    epoch: u64,
    expanded: bool,
    /// Values typed or dragged but not sent yet.
    buffer: FxHashMap<String, Val>,
    texts: FxHashMap<String, String>,
    live: bool,
    edit_text: Option<EditText>,
}

impl PropsPanel {
    #[must_use]
    pub fn new(editor: &Editor, target: usize) -> Self {
        Self {
            target,
            query: Query::new(editor.exec.clone()),
            search: String::new(),
            synced: String::new(),
            epoch: 0,
            expanded: true,
            buffer: FxHashMap::default(),
            texts: FxHashMap::default(),
            live: false,
            edit_text: None,
        }
    }

    /// Expand All (`true`) or Collapse All.
    pub fn fold_all(&mut self, open: bool) {
        self.expanded = open;
        self.epoch += 1;
    }

    /// The panel's body. `chooser` is where a pick field opens its chooser.
    pub fn draw(&mut self, ui: &mut Ui, editor: &Arc<Editor>, chooser: &mut Option<Chooser>) {
        let Some(session) = editor.project_opt() else { return };
        let Some(svc) = session.services.get::<SceneService>() else {
            state::empty_state(
                ui,
                Icon::Level,
                "No scene",
                "Open or create a scene to see its properties here.",
                None,
            );
            return;
        };
        let t = self.target;
        // the search follows the view's filter, which MCP moves too
        let view_filter = svc.view.lock().filter.get(t).cloned().unwrap_or_default();
        if view_filter != self.synced {
            self.synced.clone_from(&view_filter);
            self.search.clone_from(&view_filter);
        }
        let snap = svc.store.snapshot();
        let rev = snap.rev.mix(svc.library.rev());
        let key = (t, self.search.clone());
        let (s, lib, filter) = (snap.clone(), svc.library.clone(), self.search.clone());
        let ctx = svc.prop_cx();
        let rev = rev.mix(gz_base::Rev(Arc::as_ptr(&ctx) as usize as u64));
        let answer = self.query.get(&key, rev, move || prepare(&s, &lib, t, &filter, &ctx));
        let Some(prep) = answer.value().cloned() else {
            state::busy_line(ui, "Reading the properties...");
            return;
        };
        if t == SELECTION && prep.total == 0 && self.search.is_empty() {
            state::empty_state(
                ui,
                Icon::Select,
                "Nothing selected",
                "Select an object in the viewport or the World Outliner to see its properties here.",
                None,
            );
            return;
        }
        local_panel_header(ui, prep.icon, &prep.name, &prep.kind);
        ui.add_space(4.0);
        self.search_row(ui, &svc, prep.total);
        ui.add_space(4.0);

        let footer_h = 8.0 * 2.0 + Font::Small.line() * 2.0 + 2.0;
        let grid_h = (ui.available_height() - footer_h).max(40.0);
        let mut commits: Vec<Commit> = Vec::new();
        let mut pick: Option<(String, String, props::Choice, String)> = None;
        let mut open_text: Option<(String, String, String)> = None;
        let role = if t == WORLD { "world" } else { "selection" };
        let frac = if t == WORLD { 0.50 } else { 0.40 };
        let epoch = self.epoch;
        let expanded = self.expanded;
        egui::ScrollArea::vertical()
            .id_salt(("gz-ui-level.props", t))
            .max_height(grid_h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.push_id(epoch, |ui| {
                    PropertyGrid::new(role).label_frac(frac).show(ui, |g| {
                        let mut cx = RowCx {
                            panel: self,
                            commits: &mut commits,
                            pick: &mut pick,
                            open_text: &mut open_text,
                            expanded,
                        };
                        for n in &prep.nodes {
                            draw_node(g, n, 0, &mut cx);
                        }
                    });
                    if prep.visible == 0 && prep.total > 0 {
                        state::empty_state(
                            ui,
                            Icon::Search,
                            "Nothing matches",
                            "No property passes the search.",
                            None,
                        );
                    }
                });
            });
        grid::help_footer(ui);

        // what waited for the pointer goes once it lets go; a drag of a transform ends its step
        let pointer_down = ui.input(|i| i.pointer.any_down());
        if !pointer_down {
            for (key, v) in std::mem::take(&mut self.buffer) {
                commits.push(Commit { key, text: val_text(&v), live: false });
            }
            if self.live {
                self.live = false;
                svc.store.seal();
            }
        }
        for c in commits {
            if c.live {
                self.live = true;
            }
            send(editor, t, c);
        }
        if let Some((key, label, choice, current)) = pick {
            if t == SELECTION {
                let mut v = svc.view.lock();
                v.choosing = Some(key.clone());
                v.chosen.clear();
            }
            let items = owner_items(&snap, &choice);
            let list = ListKey::of_choose(&choice);
            let mut c = several(
                Chooser::new(Purpose::Property { target: t, key: key.clone() }, &label, list, &current),
                &key,
                &choice,
            );
            if let Some(items) = items {
                c = c.with_items(items);
            }
            *chooser = Some(c);
        }
        if let Some((key, label, text)) = open_text {
            let ids = snap.selected_in_order();
            self.edit_text = Some(EditText { key, label, text, serial_target: ids });
        }
        self.draw_edit_text(ui, editor, &snap);
    }

    fn search_row(&mut self, ui: &mut Ui, svc: &SceneService, total: usize) {
        let t = self.target;
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 30.0).max(80.0);
            let hint = format!("Search {total} properties");
            let resp = SearchField::new(&mut self.search)
                .hint(&hint)
                .width(w)
                .id_salt(if t == WORLD { "props_world" } else { "props_selection" })
                .show(ui);
            if resp.changed() {
                if let Some(f) = svc.view.lock().filter.get_mut(t) {
                    f.clone_from(&self.search);
                }
                self.synced.clone_from(&self.search);
            }
            let more = icon_button(ui, Icon::More, "Expand or collapse every category", false, true);
            let popup = Id::unique(("gz-ui-level.props_more", t));
            if more.clicked() {
                egui::Popup::toggle_id(ui.ctx(), popup);
            }
            let layer = ui.layer_id();
            let mut fold = None;
            gz_ui::menu::menu_popup(ui.ctx(), popup, more.rect, layer, |ui| {
                if gz_ui::menu_item(
                    ui,
                    "Expand All",
                    gz_ui::MenuItem { icon: Icon::ExpandAll, ..gz_ui::MenuItem::default() },
                ) {
                    fold = Some(true);
                }
                if gz_ui::menu_item(
                    ui,
                    "Collapse All",
                    gz_ui::MenuItem { icon: Icon::CollapseAll, ..gz_ui::MenuItem::default() },
                ) {
                    fold = Some(false);
                }
            });
            if let Some(open) = fold {
                self.fold_all(open);
            }
        });
    }

    fn draw_edit_text(&mut self, ui: &mut Ui, editor: &Arc<Editor>, snap: &SceneState) {
        let Some(et) = self.edit_text.as_mut() else { return };
        let mut open = true;
        let mut ok = false;
        let mut cancelled = false;
        let title = format!("Edit {}", et.label);
        Dialog::new(&title, "dlg.edit_text", DialogSize::M).show(
            ui,
            &mut open,
            |ui| {
                let w = ui.available_width();
                let edit = egui::TextEdit::multiline(&mut et.text)
                    .font(Font::Mono.id())
                    .desired_width(w)
                    .min_size(Vec2::new(w, 260.0));
                ui.add_sized(Vec2::new(w, 260.0), edit);
            },
            |f| {
                if f.button("OK", Kind::Primary, true, true, "") {
                    ok = true;
                }
                if f.button("Cancel", Kind::Secondary, false, true, "") {
                    cancelled = true;
                }
            },
        );
        if ok {
            let now: Vec<_> = snap.selected_in_order();
            if self.target == WORLD || now == et.serial_target {
                send(editor, self.target, Commit { key: et.key.clone(), text: et.text.clone(), live: false });
            } else {
                editor.log.warn(format!("! {}: the text was not applied, its row is gone", et.key));
            }
            self.edit_text = None;
        } else if !open || cancelled {
            self.edit_text = None;
        }
    }

    /// `property_inspector`'s view of this panel: the filter it shows.
    #[must_use]
    pub fn search(&self) -> &str {
        &self.search
    }
}

/// A value as the text `property_set` takes.
fn val_text(v: &Val) -> String {
    match v {
        Val::None => String::new(),
        Val::Text(t) => t.clone(),
        Val::Bool(b) => b.to_string(),
        Val::Num(n) => grid::format_number(*n, 6),
        Val::Vec3(v) => vec_text([v.x, v.y, v.z]),
        Val::Color(c) => format!("0x{c:08X}"),
        Val::FColor(c) => format!("#{:06X}", fcolor_argb(*c) & 0x00ff_ffff),
    }
}

/// What a row hands back while the grid draws.
struct RowCx<'a> {
    panel: &'a mut PropsPanel,
    commits: &'a mut Vec<Commit>,
    pick: &'a mut Option<(String, String, props::Choice, String)>,
    open_text: &'a mut Option<(String, String, String)>,
    expanded: bool,
}

fn draw_node(g: &mut grid::Grid<'_>, n: &Node, depth: u32, cx: &mut RowCx<'_>) {
    match n {
        Node::Row(r) => draw_row(g, r, depth, cx),
        Node::Section(title, children) => {
            let open = cx.expanded;
            g.section(title, open, None, |g| {
                for c in children {
                    draw_node(g, c, depth + 1, cx);
                }
            });
        }
    }
}

fn help_of(row: &MergedRow) -> String {
    props::help_of(&row.row)
}

fn unit_of(row: &MergedRow) -> String {
    props::unit_of(&row.row)
}

fn draw_row(g: &mut grid::Grid<'_>, r: &MergedRow, depth: u32, cx: &mut RowCx<'_>) {
    let target = cx.panel.target;
    let key = r.row.key.clone();
    let label = r.row.label.clone();
    let help = help_of(r);
    let unit = unit_of(r);
    let mut flags = PropFlags::NONE;
    if !r.row.enabled || matches!(r.row.kind, RowKind::Caption) {
        flags = flags | PropFlags::READ_ONLY;
    }
    if r.mixed {
        flags = flags | PropFlags::MIXED;
    }
    if matches!(r.row.kind, RowKind::Bool | RowKind::Flag) {
        flags = flags | PropFlags::INLINE;
    }
    let value = cx.panel.buffer.get(&key).cloned().unwrap_or_else(|| r.row.value.clone());
    let enabled = r.row.enabled;
    let mut edited: Option<Val> = None;
    let mut commit_now: Option<String> = None;
    let mut live = false;
    g.row(&label, &help, flags, depth, |ui| {
        if !enabled {
            ui.disable();
        }
        match (&r.row.kind, &value) {
            (RowKind::Caption, _) => {
                let text = r.row.text();
                if target == WORLD && key == "file" && text.is_empty() {
                    gz_ui::text::label(
                        ui,
                        "Not saved yet",
                        Font::Body,
                        gz_ui::palette(ui.ctx()).text_muted,
                        Clip::End,
                    );
                } else {
                    grid::read_only(ui, &text, if key.ends_with("Name") { Clip::Start } else { Clip::End });
                }
            }
            (RowKind::Bool | RowKind::Flag, Val::Bool(b)) => {
                let mut v = *b;
                if grid::switch(ui, &mut v).changed() {
                    commit_now = Some(v.to_string());
                }
            }
            (RowKind::Float { .. } | RowKind::Angle { .. } | RowKind::Int { .. }, Val::Num(n)) => {
                let angle = matches!(r.row.kind, RowKind::Angle { .. });
                let shown = if angle { n.to_degrees() } else { *n };
                let resp = if matches!(r.row.kind, RowKind::Int { .. }) {
                    let mut v = shown.round() as i32;
                    let resp = grid::integer(ui, &mut v, 0.1, &unit);
                    if resp.changed() {
                        edited = Some(Val::Num(f64::from(v)));
                    }
                    resp
                } else {
                    let mut v = shown as f32;
                    let resp = grid::number(ui, &mut v, 0.01, &unit);
                    if resp.changed() {
                        edited = Some(Val::Num(if angle { f64::from(v).to_radians() } else { f64::from(v) }));
                    }
                    resp
                };
                if let Some(Val::Num(v)) = &edited
                    && !resp.dragged()
                {
                    commit_now = Some(grid::format_number(if angle { v.to_degrees() } else { *v }, 6));
                    edited = None;
                }
            }
            (RowKind::Vector { .. }, Val::Vec3(v)) => {
                let mut a = [v.x, v.y, v.z];
                if grid::vec3_field(ui, &key, &mut a, 0.01) {
                    edited = Some(Val::Vec3(gz_scene::math::Vec3::new(a[0], a[1], a[2])));
                    live = is_transform(&key);
                }
            }
            (RowKind::Angle3 { .. }, Val::Vec3(v)) => {
                let mut a = [v.x.to_degrees(), v.y.to_degrees(), v.z.to_degrees()];
                if grid::vec3_field(ui, &key, &mut a, 0.5) {
                    edited = Some(Val::Vec3(gz_scene::math::Vec3::new(
                        a[0].to_radians(),
                        a[1].to_radians(),
                        a[2].to_radians(),
                    )));
                    live = is_transform(&key);
                }
            }
            (RowKind::Color, Val::Color(_) | Val::Num(_)) => {
                let argb = match &value {
                    Val::Color(c) => *c,
                    Val::Num(n) => *n as i64 as u32,
                    _ => 0,
                };
                let mut col = color_of(argb);
                if grid::color(ui, &mut col).changed() {
                    edited = Some(Val::Color(argb_of(col)));
                }
            }
            // a colour of 0 to 1 has no alpha to show
            (RowKind::FColor, Val::FColor(c)) => {
                let mut col = color_of(fcolor_argb(*c) | 0xff00_0000);
                if grid::color(ui, &mut col).changed() {
                    edited = Some(Val::Color(argb_of(col)));
                }
            }
            (RowKind::Token(items), Val::Num(n)) => {
                let names: Vec<&str> = items.iter().map(|(s, _)| s.as_str()).collect();
                let mut cur = items.iter().position(|(_, id)| *id == *n as i64).unwrap_or(0);
                if grid::combo(ui, &key, &mut cur, &names)
                    && let Some(name) = names.get(cur)
                {
                    commit_now = Some((*name).to_owned());
                }
            }
            (RowKind::Button(captions), _) => {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for c in captions {
                        if gz_ui::Button::new(c).show(ui).clicked() {
                            commit_now = Some(c.to_string());
                        }
                    }
                });
            }
            (RowKind::Choose(choice), _) => {
                let text = r.row.text();
                match grid::pick_field(ui, &key, if text == "<none>" { "" } else { &text }, true) {
                    PickAction::Browse => {
                        *cx.pick = Some((key.clone(), label.clone(), choice.clone(), text.clone()))
                    }
                    PickAction::Clear => commit_now = Some(String::new()),
                    PickAction::None => {}
                }
            }
            (RowKind::Text | RowKind::Name | RowKind::Time, _) => {
                let text = r.row.text();
                if multi_line(&key, &text) {
                    let first = text.lines().next().unwrap_or("").to_owned();
                    ui.horizontal(|ui| {
                        let w = (ui.available_width() - 56.0).max(20.0);
                        ui.allocate_ui(Vec2::new(w, gz_ui::metrics::FRAME), |ui| {
                            grid::read_only(ui, &first, Clip::End)
                        });
                        if gz_ui::Button::new("Edit").show(ui).clicked() {
                            *cx.open_text = Some((key.clone(), label.clone(), text.clone()));
                        }
                    });
                } else {
                    let buf = cx.panel.texts.entry(key.clone()).or_insert_with(|| text.clone());
                    let resp = grid::text_field(ui, buf);
                    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if (resp.lost_focus() || enter) && *buf != text {
                        commit_now = Some(buf.clone());
                    }
                    if !resp.has_focus() {
                        cx.panel.texts.remove(&key);
                    }
                }
            }
            _ => grid::read_only(ui, &r.row.text(), Clip::End),
        }
    });
    if let Some(text) = commit_now {
        cx.panel.buffer.remove(&key);
        cx.commits.push(Commit { key, text, live: false });
    } else if let Some(v) = edited {
        if live {
            cx.commits.push(Commit { key, text: val_text(&v), live: true });
        } else {
            cx.panel.buffer.insert(key, v);
        }
    }
}

/// The row a chooser asked about over MCP (`property_choose action=open`): its label, list and
/// value.
#[must_use]
pub fn chooser_for(
    state: &SceneState,
    lib: &gz_scene::library::Library,
    key: &str,
    ctx: &PropContext,
) -> Option<Chooser> {
    let all = rows(state, lib, SELECTION, ctx);
    let r = props::find(&all, key)?;
    let RowKind::Choose(choice) = &r.row.kind else { return None };
    let text = r.row.text();
    let c = Chooser::new(
        Purpose::Property { target: SELECTION, key: r.row.key.clone() },
        &r.row.label,
        ListKey::of_choose(choice),
        if text == "<none>" { "" } else { &text },
    );
    Some(several(c, &r.row.key, choice))
}

/// Whether a choose row takes several items, written as a comma list: a respawn point's
/// profiles, and a model's bones where the row takes more than one (`max_items`: Fixed bones).
#[must_use]
pub(crate) fn takes_several(key: &str, choice: &props::Choice, list: Option<&ListKey>) -> bool {
    key.ends_with(RESPAWN_PROFILE) || (choice.max > 1 && matches!(list, Some(ListKey::SkeletonBones(_))))
}

const RESPAWN_PROFILE: &str = "Respawn Point\\Profile";

/// The chooser of a row that takes several items: the bones it has are picked already.
fn several(c: Chooser, key: &str, choice: &props::Choice) -> Chooser {
    if !takes_several(key, choice, c.list.as_ref()) {
        c
    } else if key.ends_with(RESPAWN_PROFILE) {
        c.multi()
    } else {
        c.multi_from_current()
    }
}

/// The items of a chooser whose row gives them itself: the orders' choosers (`OnFillOrderPaths`,
/// `OnFillOrderTargets`) pick the scene's ways, and the player and the story ids of the scene's
/// spawns.
#[must_use]
pub fn owner_items(snap: &SceneState, choice: &props::Choice) -> Option<Vec<String>> {
    props::scene_items(snap, &choice.list)
}

/// Whether the class has a chooser-worthy target: the tools whose selection Details shows.
#[must_use]
pub fn shows_class(class: ObjClass) -> bool {
    class.is_object_tool()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str) -> MergedRow {
        MergedRow {
            row: props::Row {
                key: key.into(),
                path: key.into(),
                label: key.rsplit('\\').next().unwrap_or(key).into(),
                unit: "",
                kind: RowKind::Text,
                value: Val::Text(String::new()),
                enabled: true,
                help: String::new(),
            },
            mixed: false,
        }
    }

    #[test]
    fn a_level_lists_its_rows_before_its_categories() {
        let mut nodes = Vec::new();
        for k in ["Transform\\Position", "Name", "Transform\\Rotation", "Flags\\A", "Visible"] {
            let parts: Vec<&str> = k.split('\\').collect();
            let dirs = parts.split_last().map_or(&[][..], |(_, d)| d);
            insert(&mut nodes, dirs, row(k));
        }
        order(&mut nodes);
        let kinds: Vec<bool> = nodes.iter().map(|n| matches!(n, Node::Section(..))).collect();
        assert_eq!(kinds, vec![false, false, true, true]);
        assert!(matches!(&nodes[2], Node::Section(t, c) if t == "Transform" && c.len() == 2));
    }

    #[test]
    fn texts_of_values() {
        assert_eq!(val_text(&Val::Bool(true)), "true");
        assert_eq!(val_text(&Val::Num(1.5)), "1.5");
        assert_eq!(val_text(&Val::Color(0xff11_2233)), "0xFF112233");
        assert!(multi_line("props.custom_data", "x"));
        assert!(!multi_line("Name", "x"));
    }
}
