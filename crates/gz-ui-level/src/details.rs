//! Details (role `selection`) and Level Settings (role `world`): the header, "Search N
//! properties" with Expand / Collapse All, the rows of `gz-scene` - plain labels, units and words
//! - in the kit's sections, and a help footer.
//!
//! - **Sections**: the rows at the top (Name), then one kit `Section` per category in the order
//!   the rows came (Transform first), its open state kept in the layout store. Inside a section
//!   its sub-categories fold as the grid's own headers, and the rows most authors never touch
//!   (flag words, the editor's own switches) wait under the section's **More**.
//! - **Values**: a position, a rotation or a scale is one line of three fields under its label;
//!   a value that differs from its default has a Reset arrow; several selected objects show the
//!   rows they all have ("Multiple values" where they differ); a value from a list is a pick
//!   field that opens the chooser ([`crate::choose`]).
//!
//! The rows are prepared off the UI thread ([`prepare`], a `Query` keyed by the scene's
//! revision, the library's and the search), the scene's own lists of a chooser too. An edit is a
//! `SceneCmd`: a transform row sends merged undo steps while it is dragged and seals the gesture
//! when the pointer lets go; any other value is sent once, when the drag or the typing ends.

use std::sync::Arc;

use egui::{Color32, Id, Ui, Vec2};
use gz_base::SmolStr;
use gz_editor::Editor;
use gz_entities::PropContext;
use gz_query::Query;
use gz_scene::format::fields;
use gz_scene::props::{self, Effect, Kind as RowKind, MergedRow, PropCx, Row, Val};
use gz_scene::{ObjClass, SceneCmd, SceneService, SceneState};
use gz_ui::grid::{self, PickAction, PropFlags, PropertyGrid};
use gz_ui::text::Clip;
use gz_ui::{Dialog, DialogSize, Font, Icon, Kind, SearchField, Section, Tone, icon_button, state};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::choose::{Chooser, Purpose};
use crate::layout::PanelId;
use crate::lists::ListKey;
use crate::local::local_panel_header;

/// Details' rows (0) or Level Settings' (1).
pub const SELECTION: usize = 0;
pub const WORLD: usize = 1;

/// A row as the panel draws it: the row, and its default when the value differs from it.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub row: MergedRow,
    /// The text that puts the default back (the Reset arrow), when the value is not it.
    pub reset: Option<String>,
}

/// A level of the grid: its own rows, then its categories, each in the order met.
#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Row(Item),
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

/// The rows of the selection - only those every selected object has, "mixed" where their
/// values differ - or of the level.
#[must_use]
pub fn rows(
    state: &SceneState,
    lib: &gz_scene::library::Library,
    target: usize,
    ctx: &PropContext,
) -> Vec<MergedRow> {
    if target == WORLD {
        return gz_scene::selection_rows(state, lib, true, ctx);
    }
    let cx = PropCx { lib: Some(lib), entity: ctx };
    let lists: Vec<Vec<Row>> = state
        .selected_in_order()
        .iter()
        .filter_map(|id| state.get(*id))
        .map(|o| props::object_rows(state, o, &cx))
        .collect();
    let n = lists.len();
    let mut have: FxHashMap<String, usize> = FxHashMap::default();
    for list in &lists {
        let keys: FxHashSet<&str> = list.iter().map(|r| r.key.as_str()).collect();
        for k in keys {
            *have.entry(k.to_owned()).or_default() += 1;
        }
    }
    let shared = props::merge(lists).into_iter().filter(|m| have.get(&m.row.key) == Some(&n)).collect();
    props::tree_order(shared)
}

fn insert(nodes: &mut Vec<Node>, path: &[&str], item: Item) {
    match path.split_first() {
        None => nodes.push(Node::Row(item)),
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
                insert(children, rest, item);
            }
        }
    }
}

/// Where a top category stands: Transform first, the thing's own next, what the module and the
/// editor keep of it last.
fn top_rank(title: &str) -> u8 {
    match title {
        "Transform" => 0,
        "Module" => 2,
        "Editor" => 3,
        _ => 1,
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

/// The one kind the selection is of, for the defaults.
fn one_class(state: &SceneState) -> Option<ObjClass> {
    let mut kinds = state.selected_in_order().into_iter().filter_map(|id| state.get(id)).map(|o| o.class);
    let first = kinds.next()?;
    kinds.all(|k| k == first).then_some(first)
}

/// The default of a key as the scene's reference writes it.
fn default_of(state: &SceneState, target: usize, key: &str) -> Option<&'static str> {
    if target == WORLD {
        return gz_scene::format::SCENE_FIELDS.iter().find(|f| f.key == key).map(|f| f.default);
    }
    fields::object_field(one_class(state)?, key).map(|f| f.default)
}

/// The numbers of a default written as a table: `{ 1, 0, 0 }`.
fn numbers(text: &str) -> Vec<f64> {
    text.trim()
        .trim_start_matches('{')
        .trim_end_matches('}')
        .split(',')
        .filter_map(|x| x.trim().parse().ok())
        .collect()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-4 * a.abs().max(b.abs()).max(1.0)
}

/// The text that puts a row's default back, when its value is not the default; `None` when it
/// is, or when the row has no default to go back to.
#[must_use]
pub fn reset_text(row: &Row, default: &str) -> Option<String> {
    let d = default.trim();
    let quoted = d.trim_matches('"');
    let differs_to = |same: bool, text: String| (!same).then_some(text);
    match (&row.kind, &row.value) {
        (RowKind::Bool | RowKind::Flag, Val::Bool(b)) => {
            let want = match d {
                "true" => true,
                "false" => false,
                _ => return None,
            };
            differs_to(*b == want, want.to_string())
        }
        (RowKind::Float { .. } | RowKind::Int { .. }, Val::Num(n)) => {
            let want: f64 = d.parse().ok()?;
            differs_to(close(*n, want), grid::format_number(want, 6))
        }
        (RowKind::Angle { .. }, Val::Num(n)) => {
            let want: f64 = d.parse().ok()?;
            differs_to(close(n.to_degrees(), want), grid::format_number(want, 6))
        }
        (RowKind::Vector { .. } | RowKind::Angle3 { .. }, Val::Vec3(v)) => {
            let want = numbers(d);
            let [x, y, z] = <[f64; 3]>::try_from(want).ok()?;
            let deg = matches!(row.kind, RowKind::Angle3 { .. });
            let have = [v.x, v.y, v.z].map(|c| if deg { f64::from(c).to_degrees() } else { f64::from(c) });
            let same = close(have[0], x) && close(have[1], y) && close(have[2], z);
            differs_to(
                same,
                format!(
                    "{}, {}, {}",
                    grid::format_number(x, 6),
                    grid::format_number(y, 6),
                    grid::format_number(z, 6)
                ),
            )
        }
        (RowKind::FColor, Val::FColor(c)) => {
            let want = numbers(d);
            let [r, g, b] = <[f64; 3]>::try_from(want).ok()?;
            let same = close(f64::from(c[0]), r) && close(f64::from(c[1]), g) && close(f64::from(c[2]), b);
            let hex = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
            differs_to(same, format!("#{:02X}{:02X}{:02X}", hex(r), hex(g), hex(b)))
        }
        (RowKind::Token(items), Val::Num(n)) => {
            let want = props::label_of(quoted);
            let name = items.iter().find(|(_, id)| *id == *n as i64).map(|(s, _)| s.as_str())?;
            let known = items.iter().any(|(s, _)| s.eq_ignore_ascii_case(&want));
            if !known {
                return None;
            }
            differs_to(name.eq_ignore_ascii_case(&want), want)
        }
        (RowKind::Text | RowKind::Choose(_), Val::Text(t)) if quoted.is_empty() || d == "none" => {
            differs_to(t.is_empty() || t == "<none>", String::new())
        }
        _ => None,
    }
}

/// The rows as a tree of categories, filtered by `filter`, with the header.
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
        let reset = if r.mixed || !r.row.enabled {
            None
        } else {
            default_of(state, target, &r.row.key).and_then(|d| reset_text(&r.row, d))
        };
        let parts: Vec<&str> = r.row.path.split('\\').collect();
        let dirs = parts.split_last().map_or(&[][..], |(_, d)| d);
        insert(&mut nodes, dirs, Item { row: r.clone(), reset });
    }
    order(&mut nodes);
    nodes.sort_by_key(|n| match n {
        Node::Row(_) => 0,
        Node::Section(t, _) => 1 + top_rank(t),
    });
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

/// One object names itself, several say how many and of what kind.
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
                if uniform {
                    first.desc().to_owned()
                } else {
                    "Several kinds - the rows they share".to_owned()
                },
            )
        }
    }
}

/// "Search 1 property", "Search 30 properties".
#[must_use]
pub fn search_hint(total: usize) -> String {
    if total == 1 { "Search 1 property".to_owned() } else { format!("Search {total} properties") }
}

/// Rows most authors never touch: under their section's More.
#[must_use]
pub fn advanced(key: &str) -> bool {
    matches!(
        key,
        "virtual_size"
            | "fuzzy"
            | "light_flags"
            | "glow_flags"
            | "object_flags"
            | "sound_flags"
            | "spawn_flags"
            | "group_editable"
            | "draw_selected"
            | "flags"
            | "game_types"
            | "vertices"
    ) || key.starts_with("fuzzy.")
        || key.ends_with(".raw")
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

/// An edit to send: the row, its value as text, and whether it is a live step of a drag (merged
/// into one undo step).
#[derive(Clone, Debug, PartialEq)]
struct Commit {
    key: String,
    text: String,
    live: bool,
}

/// Applies a row's value given as text as a `SceneCmd`; a transform row's live steps merge under
/// `merge`.
pub async fn set_value(
    editor: &Arc<Editor>,
    target: usize,
    key: &str,
    text: &str,
    merge: Option<SmolStr>,
) -> Result<(), String> {
    let svc = SceneService::of(editor).map_err(|e| e.message)?;
    let snap = svc.store.snapshot();
    let ctx = svc.prop_cx();
    let (s2, lib) = (snap.clone(), svc.library.clone());
    let c2 = ctx.clone();
    let all = editor.exec.cpu(move || rows(&s2, &lib, target, &c2)).await;
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

/// The layout store's key of a section's fold.
fn section_key(target: usize, title: &str) -> String {
    format!("level.{}.{}", if target == WORLD { "world" } else { "details" }, title.to_lowercase())
}

/// One of the two panels.
pub struct PropsPanel {
    pub target: usize,
    query: Query<(usize, String), Prepared>,
    search: String,
    /// The filter as last read from the scene's view.
    synced: String,
    /// The grid's own sub-sections fold anew when this moves (Expand / Collapse All).
    epoch: u64,
    expanded: bool,
    /// The sections drawn last, for Expand / Collapse All.
    sections: Vec<String>,
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
            sections: Vec::new(),
            buffer: FxHashMap::default(),
            texts: FxHashMap::default(),
            live: false,
            edit_text: None,
        }
    }

    /// Expand All (`true`) or Collapse All: every section and every category in them.
    pub fn fold_all(&mut self, ctx: &egui::Context, open: bool) {
        self.expanded = open;
        self.epoch += 1;
        for title in &self.sections {
            gz_ui::section::set_open(ctx, &section_key(self.target, title), open);
        }
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
        // the search follows the view's filter, which a command moves too
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
            let (title, text) = if snap.selection.is_empty() {
                (
                    "Nothing selected",
                    "Select an object in the viewport or the World Outliner to see its properties here.",
                )
            } else {
                ("Nothing shared", "The selected objects have no property in common.")
            };
            state::empty_state(ui, Icon::Select, title, text, None);
            gz_app::report_scroll(ui.ctx(), self.window(), 0.0, 0.0);
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
        let mut sections = Vec::new();
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("gz-ui-level.props", t))
            .max_height(grid_h)
            .auto_shrink([false, false]);
        match gz_app::take_scroll(ui.ctx(), self.window()) {
            Some(gz_app::ScrollTo::Top) => scroll = scroll.vertical_scroll_offset(0.0),
            Some(gz_app::ScrollTo::Bottom) => scroll = scroll.vertical_scroll_offset(f32::MAX / 4.0),
            Some(gz_app::ScrollTo::Offset(y)) => scroll = scroll.vertical_scroll_offset(y),
            _ => {}
        }
        let window = self.window();
        let out = scroll.show(ui, |ui| {
            ui.push_id(epoch, |ui| {
                let mut cx = RowCx {
                    panel: self,
                    commits: &mut commits,
                    pick: &mut pick,
                    open_text: &mut open_text,
                    expanded,
                };
                let top: Vec<&Node> = prep.nodes.iter().filter(|n| matches!(n, Node::Row(_))).collect();
                if !top.is_empty() {
                    ui.push_id("top", |ui| {
                        PropertyGrid::new(role).label_frac(frac).show(ui, |g| {
                            for n in &top {
                                draw_node(g, n, 0, &mut cx);
                            }
                        });
                    });
                }
                for n in &prep.nodes {
                    let Node::Section(title, children) = n else { continue };
                    sections.push(title.clone());
                    let count = count_rows(children).to_string();
                    let key = section_key(t, title);
                    let open = Section::new(&key, title).count(&count).show(ui).open;
                    if !open {
                        continue;
                    }
                    let (plain, more): (Vec<&Node>, Vec<&Node>) =
                        children.iter().partition(|c| !matches!(c, Node::Row(i) if advanced(&i.row.row.key)));
                    ui.push_id(title.as_str(), |ui| {
                        PropertyGrid::new(role).label_frac(frac).show(ui, |g| {
                            for c in &plain {
                                draw_node(g, c, 0, &mut cx);
                            }
                            if !more.is_empty() {
                                let badge = more.len().to_string();
                                g.section("More", false, Some(&badge), |g| {
                                    for c in &more {
                                        draw_node(g, c, 1, &mut cx);
                                    }
                                });
                            }
                        });
                    });
                    ui.add_space(4.0);
                }
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
        gz_app::report_scroll(
            ui.ctx(),
            window,
            out.state.clamped_offset().y,
            (out.content_size.y - out.inner_rect.height()).max(0.0),
        );
        self.sections = sections;
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
            *chooser = Some(chooser_of(t, &key, &label, &choice, &current));
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
            let hint = search_hint(total);
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
            let more = icon_button(ui, Icon::More, "Expand or collapse every section", false, true);
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
                self.fold_all(ui.ctx(), open);
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
                editor.log.warn(format!("{}: the text was not applied, the selection changed", et.label));
            }
            self.edit_text = None;
        } else if !open || cancelled {
            self.edit_text = None;
        }
    }

    /// The panel's window, as the contract names it (`ui.scroll id=details`).
    fn window(&self) -> &'static str {
        let panel = if self.target == WORLD { PanelId::LevelSettings } else { PanelId::Details };
        panel.window()
    }

    /// The search the panel shows.
    #[must_use]
    pub fn search(&self) -> &str {
        &self.search
    }

    /// The sections the panel drew last.
    #[must_use]
    pub fn sections(&self) -> &[String] {
        &self.sections
    }
}

fn count_rows(nodes: &[Node]) -> usize {
    nodes
        .iter()
        .map(|n| match n {
            Node::Row(_) => 1,
            Node::Section(_, c) => count_rows(c),
        })
        .sum()
}

/// A value as the text a row parses.
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

/// What a row's value does this frame.
#[derive(Default)]
struct Edited {
    value: Option<Val>,
    now: Option<String>,
    live: bool,
}

/// Three numbers tagged X, Y, Z: a position, a scale, or a rotation in degrees.
fn vector(ui: &mut Ui, r: &MergedRow, value: &Val, out: &mut Edited) {
    let Val::Vec3(v) = value else { return };
    let key = &r.row.key;
    if matches!(r.row.kind, RowKind::Angle3 { .. }) {
        let mut a = [v.x.to_degrees(), v.y.to_degrees(), v.z.to_degrees()];
        if grid::vec3_field(ui, key, &mut a, 0.5) {
            out.value = Some(Val::Vec3(gz_scene::math::Vec3::new(
                a[0].to_radians(),
                a[1].to_radians(),
                a[2].to_radians(),
            )));
            out.live = is_transform(key);
        }
    } else {
        let mut a = [v.x, v.y, v.z];
        if grid::vec3_field(ui, key, &mut a, 0.01) {
            out.value = Some(Val::Vec3(gz_scene::math::Vec3::new(a[0], a[1], a[2])));
            out.live = is_transform(key);
        }
    }
}

#[allow(clippy::too_many_lines)]
fn draw_row(g: &mut grid::Grid<'_>, item: &Item, depth: u32, cx: &mut RowCx<'_>) {
    let r = &item.row;
    let target = cx.panel.target;
    let key = r.row.key.clone();
    let label = r.row.label.clone();
    let help = props::help_of(&r.row);
    let unit = props::unit_of(&r.row);
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
    if item.reset.is_some() {
        flags = flags | PropFlags::MODIFIED;
    }
    let value = cx.panel.buffer.get(&key).cloned().unwrap_or_else(|| r.row.value.clone());
    let enabled = r.row.enabled;
    let mut out = Edited::default();
    // a vector takes the line under its label: three fields side by side, never stacked
    let vector_row = matches!(r.row.kind, RowKind::Vector { .. } | RowKind::Angle3 { .. })
        && matches!(value, Val::Vec3(_))
        && !g.stacked();
    g.row(&label, &help, flags, depth, |ui| {
        if !enabled {
            ui.disable();
        }
        if vector_row {
            return;
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
                    grid::read_only(ui, &text, if key == "file" { Clip::Start } else { Clip::End });
                }
            }
            (RowKind::Bool | RowKind::Flag, Val::Bool(b)) => {
                let mut v = *b;
                if grid::switch(ui, &mut v).changed() {
                    out.now = Some(v.to_string());
                }
            }
            (RowKind::Float { .. } | RowKind::Angle { .. } | RowKind::Int { .. }, Val::Num(n)) => {
                let angle = matches!(r.row.kind, RowKind::Angle { .. });
                let shown = if angle { n.to_degrees() } else { *n };
                let resp = if matches!(r.row.kind, RowKind::Int { .. }) {
                    let mut v = shown.round() as i32;
                    let resp = grid::integer(ui, &mut v, 0.1, &unit);
                    if resp.changed() {
                        out.value = Some(Val::Num(f64::from(v)));
                    }
                    resp
                } else {
                    let mut v = shown as f32;
                    let resp = grid::number(ui, &mut v, 0.01, &unit);
                    if resp.changed() {
                        out.value =
                            Some(Val::Num(if angle { f64::from(v).to_radians() } else { f64::from(v) }));
                    }
                    resp
                };
                if let Some(Val::Num(v)) = &out.value
                    && !resp.dragged()
                {
                    out.now = Some(grid::format_number(if angle { v.to_degrees() } else { *v }, 6));
                    out.value = None;
                }
            }
            (RowKind::Vector { .. } | RowKind::Angle3 { .. }, Val::Vec3(_)) => {
                vector(ui, r, &value, &mut out)
            }
            (RowKind::Color, Val::Color(_) | Val::Num(_)) => {
                let argb = match &value {
                    Val::Color(c) => *c,
                    Val::Num(n) => *n as i64 as u32,
                    _ => 0,
                };
                let mut col = color_of(argb);
                if grid::color(ui, &mut col).changed() {
                    out.value = Some(Val::Color(argb_of(col)));
                }
            }
            // a colour of 0 to 1 has no alpha to show
            (RowKind::FColor, Val::FColor(c)) => {
                let mut col = color_of(fcolor_argb(*c) | 0xff00_0000);
                if grid::color(ui, &mut col).changed() {
                    out.value = Some(Val::Color(argb_of(col)));
                }
            }
            (RowKind::Token(items), Val::Num(n)) => {
                let names: Vec<&str> = items.iter().map(|(s, _)| s.as_str()).collect();
                let mut cur = items.iter().position(|(_, id)| *id == *n as i64).unwrap_or(0);
                if grid::combo(ui, &key, &mut cur, &names)
                    && let Some(name) = names.get(cur)
                {
                    out.now = Some((*name).to_owned());
                }
            }
            (RowKind::Button(captions), _) => {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for c in captions {
                        if gz_ui::Button::new(c).show(ui).clicked() {
                            out.now = Some(c.to_string());
                        }
                    }
                });
            }
            (RowKind::Choose(choice), _) => {
                let text = r.row.text();
                match grid::pick_field(ui, &key, if text == "<none>" { "" } else { &text }, true) {
                    PickAction::Browse => {
                        *cx.pick = Some((key.clone(), label.clone(), choice.clone(), text.clone()));
                    }
                    PickAction::Clear => out.now = Some(String::new()),
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
                        out.now = Some(buf.clone());
                    }
                    if !resp.has_focus() {
                        cx.panel.texts.remove(&key);
                    }
                }
            }
            _ => grid::read_only(ui, &r.row.text(), Clip::End),
        }
    });
    if g.was_reset()
        && let Some(text) = &item.reset
    {
        out.now = Some(text.clone());
    }
    if vector_row {
        let indent = 12.0 * depth as f32 + 6.0;
        g.block(|ui| {
            ui.horizontal(|ui| {
                ui.add_space(indent);
                let w = ui.available_width() - 30.0;
                ui.allocate_ui(Vec2::new(w, gz_ui::metrics::FRAME), |ui| {
                    if !enabled {
                        ui.disable();
                    }
                    vector(ui, r, &value, &mut out);
                });
            });
        });
    }
    if let Some(text) = out.now {
        cx.panel.buffer.remove(&key);
        cx.commits.push(Commit { key, text, live: false });
    } else if let Some(v) = out.value {
        if out.live {
            cx.commits.push(Commit { key, text: val_text(&v), live: true });
        } else {
            cx.panel.buffer.insert(key, v);
        }
    }
}

/// The chooser of a choose row: its list, the value it starts from, the items it takes. A list
/// the scene itself gives (its way paths, its story ids) is read off the UI thread.
#[must_use]
pub fn chooser_of(target: usize, key: &str, label: &str, choice: &props::Choice, current: &str) -> Chooser {
    let list = ListKey::of_choose(choice);
    let scene = list.is_none();
    let current = if current == "<none>" { "" } else { current };
    let mut c = several(
        Chooser::new(Purpose::Property { target, key: key.to_owned() }, label, list, current),
        key,
        choice,
    );
    if scene {
        c = c.with_scene_list(&choice.list);
    }
    c
}

/// The chooser of a row of the selection's, by its key: `None` for a row that picks nothing.
#[must_use]
pub fn chooser_for(rows: &[MergedRow], key: &str, target: usize) -> Option<Chooser> {
    let r = props::find(rows, key)?;
    let RowKind::Choose(choice) = &r.row.kind else { return None };
    Some(chooser_of(target, &r.row.key, &r.row.label, choice, &r.row.text()))
}

/// Whether a choose row takes several items, written as a comma list: a respawn point's
/// profiles, and a model's bones where the row takes more than one (Fixed Bones).
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

/// Whether the class has a chooser-worthy target: the tools whose selection Details shows.
#[must_use]
pub fn shows_class(class: ObjClass) -> bool {
    class.is_object_tool()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, kind: RowKind, value: Val) -> Row {
        Row {
            key: key.into(),
            path: key.into(),
            label: key.rsplit('\\').next().unwrap_or(key).into(),
            unit: "",
            kind,
            value,
            enabled: true,
            help: String::new(),
        }
    }

    fn item(key: &str) -> Item {
        Item {
            row: MergedRow { row: row(key, RowKind::Text, Val::Text(String::new())), mixed: false },
            reset: None,
        }
    }

    #[test]
    fn a_level_lists_its_rows_before_its_categories() {
        let mut nodes = Vec::new();
        for k in ["Transform\\Position", "Name", "Transform\\Rotation", "Flags\\A", "Visible"] {
            let parts: Vec<&str> = k.split('\\').collect();
            let dirs = parts.split_last().map_or(&[][..], |(_, d)| d);
            insert(&mut nodes, dirs, item(k));
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
        assert_eq!(search_hint(1), "Search 1 property");
        assert_eq!(search_hint(30), "Search 30 properties");
    }

    #[test]
    fn a_value_off_its_default_resets_to_it() {
        let range = row("range", RowKind::Float { min: 0.0, max: 1000.0, dec: 2 }, Val::Num(12.0));
        assert_eq!(reset_text(&range, "8").as_deref(), Some("8"));
        let at = row("range", RowKind::Float { min: 0.0, max: 1000.0, dec: 2 }, Val::Num(8.0));
        assert_eq!(reset_text(&at, "8"), None);
        let pos = row(
            "pos",
            RowKind::Vector { min: -1e6, max: 1e6, dec: 3 },
            Val::Vec3(gz_scene::math::Vec3::new(1.0, 0.0, 0.0)),
        );
        assert_eq!(reset_text(&pos, "{ 0, 0, 0 }").as_deref(), Some("0, 0, 0"));
        let cone = row("cone", RowKind::Angle { min: 0.0, max: 3.0, dec: 1 }, Val::Num(22.5f64.to_radians()));
        assert_eq!(reset_text(&cone, "22.5"), None);
        let on = row("viewport", RowKind::Bool, Val::Bool(false));
        assert_eq!(reset_text(&on, "true").as_deref(), Some("true"));
        let kind = row("kind", RowKind::Token(vec![("Point".into(), 1), ("Spot".into(), 2)]), Val::Num(2.0));
        assert_eq!(reset_text(&kind, "\"point\"").as_deref(), Some("Point"));
        // a default the row cannot be compared with gives no arrow
        assert_eq!(reset_text(&range, "radius 10, power 1"), None);
        assert!(advanced("light_flags") && !advanced("range"));
    }

    /// A position draws on the line under its label: one line of three fields, the X, Y and Z
    /// tags side by side, in a grid as wide as Details is in the Wide layout.
    #[test]
    fn a_vector_takes_one_line_under_its_label() {
        let ctx = egui::Context::default();
        gz_ui::theme::install(&ctx, gz_ui::theme::Accent::default());
        let pos = Item {
            row: MergedRow {
                row: row(
                    "pos",
                    RowKind::Vector { min: -1e6, max: 1e6, dec: 3 },
                    Val::Vec3(gz_scene::math::Vec3::new(1.0, 2.0, 3.0)),
                ),
                mixed: false,
            },
            reset: Some("0, 0, 0".into()),
        };
        let rt =
            gz_task::Runtime::start(gz_task::ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 1 })
                .expect("runtime");
        let ed = Editor::new(
            gz_editor::EditorOptions::default(),
            rt.exec(),
            gz_log::Log::memory(),
            gz_editor::Registry::new(),
        );
        let mut panel = PropsPanel::new(&ed, SELECTION);
        let mut heights = (0.0, 0.0);
        for _ in 0..3 {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, Vec2::new(324.0, 400.0))),
                ..egui::RawInput::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let (mut commits, mut pick, mut text) = (Vec::new(), None, None);
                let mut cx = RowCx {
                    panel: &mut panel,
                    commits: &mut commits,
                    pick: &mut pick,
                    open_text: &mut text,
                    expanded: true,
                };
                let top = ui.cursor().min.y;
                PropertyGrid::new("selection").show(ui, |g| draw_row(g, &pos, 0, &mut cx));
                let h = ui.cursor().min.y - top;
                heights = (h, gz_ui::metrics::PROPERTY_ROW);
            });
            out.textures_delta.clear();
        }
        let (h, row_h) = heights;
        // the label's row and one line of fields under it - not three stacked fields
        assert!(h > row_h * 1.5 && h < row_h * 2.6, "a position took {h} pt");
        drop(rt);
    }
}
