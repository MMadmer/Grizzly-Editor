//! What the Level tab's panels draw that no other surface does, and the one copy of what every
//! panel of the crate does with commands: [`run`] (a command off the UI thread, its answer a
//! notice), [`confirm`], [`removal`], [`args`].

use std::sync::Arc;

use egui::{Align2, CornerRadius, Id, Rect, Response, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, pos2};
use gz_editor::{Args, Editor, Value};
use gz_query::Query;
use gz_ui::text::{self, Clip};
use gz_ui::{Button, Font, Icon, Kind, TipExt, Tx, audit, icons, metrics, palette};

/// `local_eye_toggle`: an eye (or a crossed-out eye) filling `rect`, hover-filled. Returns
/// whether it was clicked.
pub fn local_eye_toggle<'a>(ui: &mut Ui, id: Id, rect: Rect, visible: bool, tip: impl Into<Tx<'a>>) -> bool {
    let icon = if visible { Icon::Eye } else { Icon::EyeOff };
    local_glyph_toggle(ui, id, rect, icon, visible, true, tip.into())
}

/// `local_lock_toggle`: a closed lock while `locked`; an open one only under the pointer, so a
/// column of unlocked rows stays quiet. Returns whether it was clicked.
pub fn local_lock_toggle<'a>(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    locked: bool,
    row_hovered: bool,
    tip: impl Into<Tx<'a>>,
) -> bool {
    let icon = if locked { Icon::Lock } else { Icon::Unlock };
    local_glyph_toggle(ui, id, rect, icon, locked, locked || row_hovered, tip.into())
}

/// A square glyph button of a list's column: bright while `on`, drawn only while `shown` or
/// hovered.
fn local_glyph_toggle(
    ui: &mut Ui,
    id: Id,
    rect: Rect,
    icon: Icon,
    on: bool,
    shown: bool,
    tip: Tx<'_>,
) -> bool {
    let p = palette(ui.ctx());
    let resp = ui.interact(rect, id, Sense::click());
    let hovered = resp.hovered();
    let held = resp.is_pointer_button_down_on();
    if ui.is_rect_visible(rect) && (shown || hovered) {
        if hovered || held {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(metrics::RADIUS),
                if held { p.pressed } else { p.hover },
            );
        }
        let tone = match (on, hovered) {
            (true, true) => p.text,
            (true, false) | (false, true) => p.text_secondary,
            (false, false) => p.text_disabled,
        };
        let g = Rect::from_center_size(rect.center(), Vec2::splat(metrics::GLYPH_ROW));
        icons::draw(ui.painter(), icon, g, tone);
    }
    // the audit reads the English; the window the language it shows
    let en = tip.en();
    audit::control(ui.ctx(), &en, true, None, Some(&en), rect);
    let clicked = resp.clicked();
    resp.tip(&tip);
    clicked
}

/// The arguments of a command from name-value pairs.
#[must_use]
pub fn args(pairs: &[(&str, Value)]) -> Args {
    let mut a = Args::new();
    for (k, v) in pairs {
        a.insert((*k).into(), v.clone());
    }
    a
}

/// Runs a command off the UI thread, as the panels' buttons do: a refusal becomes an error
/// notice; what the command says it did (`message`, `note`, the first `failed` line, a stopped
/// generation, a validation) a note.
pub fn run(editor: &Arc<Editor>, command: &'static str, a: Args) {
    let ed = editor.clone();
    let origin = editor.ui_origin();
    editor.exec.spawn(async move {
        let reply = ed.call(command, a, origin).await;
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            let text = reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.").to_owned();
            ed.notices.error(text);
            return;
        }
        if let Some(note) = reply_note(&reply) {
            ed.notices.note(note);
        }
    });
}

/// The sentence a panel shows of a command's answer, when it has one.
fn reply_note(reply: &Value) -> Option<String> {
    let s = |k: &str| reply.get(k).and_then(Value::as_str).map(str::to_owned);
    if let Some(m) = s("message").or_else(|| s("note")) {
        return Some(m);
    }
    if let Some(f) =
        reply.get("failed").and_then(Value::as_array).and_then(|f| f.first()).and_then(Value::as_str)
    {
        return Some(f.to_owned());
    }
    if reply.get("stopped").and_then(Value::as_bool) == Some(true) {
        let n = reply.get("added").and_then(Value::as_u64).unwrap_or(0);
        return Some(format!("Generation stopped: the {n} nodes made so far are kept."));
    }
    if reply.get("valid").and_then(Value::as_bool) == Some(true) {
        return Some("The cells are valid.".to_owned());
    }
    reply
        .get("problems")
        .and_then(Value::as_array)
        .and_then(|p| p.first())
        .and_then(Value::as_str)
        .map(|p| format!("{p} (the Output Log lists every problem)"))
}

/// Asks the kit's confirmation (the person, or the logged answer under `-nodlg`) and runs `then`
/// on Yes.
pub fn confirm(
    ctx: &egui::Context,
    editor: &Editor,
    spec: gz_ui::ConfirmSpec,
    then: impl FnOnce() + Send + 'static,
) {
    let line = gz_ui::dialog::confirm(ctx, crate::tools::asker(editor), spec, move |answer| {
        if answer == gz_ui::Answer::Yes {
            then();
        }
    });
    if let Some(line) = line {
        editor.log.finding(&line);
    }
}

/// The question of a removal of `items`: Cancel the default, Undo named as the way back.
#[must_use]
pub fn removal(
    id: &str,
    title: &'static str,
    headline: &'static str,
    items: Vec<gz_ui::Tx<'static>>,
    action: &'static str,
) -> gz_ui::ConfirmSpec {
    gz_ui::ConfirmSpec {
        id: id.into(),
        title: title.into(),
        headline: headline.into(),
        total: items.len(),
        items,
        consequence: "Edit > Undo brings them back.".into(),
        action: action.into(),
        destructive: true,
    }
}

/// A button of a [`local_button_flow`].
#[derive(Clone, Debug)]
pub struct LocalButton<'a> {
    pub label: Tx<'a>,
    pub tip: Tx<'a>,
    pub enabled: bool,
    /// Why it is off.
    pub why: Tx<'a>,
}

impl<'a> LocalButton<'a> {
    #[must_use]
    pub fn new(label: impl Into<Tx<'a>>, tip: impl Into<Tx<'a>>) -> Self {
        Self { label: label.into(), tip: tip.into(), enabled: true, why: Tx::EMPTY }
    }

    #[must_use]
    pub fn when(mut self, enabled: bool, why: impl Into<Tx<'a>>) -> Self {
        self.enabled = enabled;
        self.why = why.into();
        self
    }
}

/// `local_button_flow`: Secondary buttons side by side, wrapping onto the next line when the
/// row is full. Returns the index pressed.
pub fn local_button_flow(ui: &mut Ui, buttons: &[LocalButton<'_>]) -> Option<usize> {
    let mut pressed = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        for (i, b) in buttons.iter().enumerate() {
            let mut button = Button::new(&b.label).kind(Kind::Secondary).enabled(b.enabled, &b.why);
            if b.enabled {
                button = button.tip(&b.tip);
            }
            if button.show(ui).clicked() {
                pressed = Some(i);
            }
        }
    });
    pressed
}

/// `local_panel_header`: an icon, the name and, muted, what it is.
pub fn local_panel_header(ui: &mut Ui, icon: Icon, name: Tx<'_>, kind: Tx<'_>) {
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let h = metrics::SECTION_HEADER;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    let g = Rect::from_min_size(
        pos2(rect.min.x + 2.0, rect.center().y - metrics::GLYPH * 0.5),
        Vec2::splat(metrics::GLYPH),
    );
    icons::draw(painter, icon, g, p.accent_bright);
    let x = g.max.x + 8.0;
    let kind_w = if kind.is_empty() { 0.0 } else { text::width(&ctx, &Font::Small.id(), &kind) + 8.0 };
    let room = (rect.max.x - x - kind_w).max(0.0);
    let cut = text::paint(
        painter,
        &Font::Strong.id(),
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        p.text,
        &name,
        Some(room),
        Clip::End,
    );
    if cut {
        let (full, en) = (name.get(&ctx), name.en());
        audit::truncated_text(&ctx, &full, Some(&en), &full, rect);
    }
    if !kind.is_empty() {
        text::paint(
            painter,
            &Font::Small.id(),
            pos2(rect.max.x, rect.center().y),
            Align2::RIGHT_CENTER,
            p.text_muted,
            &kind,
            Some(kind_w),
            Clip::End,
        );
    }
}

/// `local_thumbnail`: a picture's place without a picture: an inset square with the kind's glyph.
pub fn local_thumbnail<'a>(ui: &mut Ui, side: f32, icon: Icon, tip: impl Into<Tx<'a>>) {
    let tip = tip.into();
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect(
            rect,
            CornerRadius::same(metrics::RADIUS),
            p.inset,
            Stroke::new(1.0, p.border_subtle),
            StrokeKind::Inside,
        );
        let g = (side * 0.4).clamp(16.0, 32.0);
        icons::draw(painter, icon, Rect::from_center_size(rect.center(), Vec2::splat(g)), p.text_muted);
    }
    if !tip.is_empty() {
        resp.tip(tip);
    }
}

/// A Ghost toolbar button with a glyph, a label and a count badge on its corner (the
/// Outliner's Filters). `checked` lights it.
pub fn local_badge_button<'a>(
    ui: &mut Ui,
    icon: Icon,
    label: impl Into<Tx<'a>>,
    tip: impl Into<Tx<'a>>,
    count: usize,
    checked: bool,
) -> Response {
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let resp = Button::new(label)
        .kind(Kind::Ghost)
        .icon(icon)
        .checked(checked)
        .min_size(Vec2::new(0.0, metrics::TOOLBAR_BUTTON))
        .tip(tip)
        .show(ui);
    if count > 0 && ui.is_rect_visible(resp.rect) {
        let t = count.to_string();
        let font = Font::Small.id();
        let w = text::width(&ctx, &font, Tx::raw(&t));
        let c = pos2(resp.rect.max.x - 3.0, resp.rect.min.y + 3.0);
        ui.painter().circle_filled(c, (w * 0.5 + 3.0).max(7.0), p.accent);
        ui.painter().text(c, Align2::CENTER_CENTER, t, font, p.on_fill);
    }
    resp
}

/// One row of a [`local_tree_list`]: a folder or an item.
#[derive(Clone, Debug, PartialEq, Eq)]
struct TreeRow {
    depth: u16,
    /// The whole path of the item, or of the folder (no trailing separator).
    path: String,
    folder: bool,
}

/// What a [`local_tree_list`] keeps between frames: its rows worked out off the UI thread
/// (a `Query` keyed by the search and the open folders, its revision the items' list), and the
/// last rows shown while new ones come.
#[derive(Clone)]
struct TreeState {
    query: Query<(String, u64), Vec<TreeRow>>,
    shown: Arc<Vec<TreeRow>>,
}

#[derive(Clone, Default)]
struct OpenFolders {
    set: rustc_hash::FxHashSet<String>,
    epoch: u64,
}

/// The rows a tree shows: folders (open ones show what they hold) and items, every folder that
/// holds a match open while a search is on.
fn tree_rows(items: &[String], search: &str, open: &rustc_hash::FxHashSet<String>) -> Vec<TreeRow> {
    let mut out: Vec<TreeRow> = Vec::new();
    // the folders already emitted, as a stack of the current path's parts
    let mut stack: Vec<String> = Vec::new();
    let searching = !search.trim().is_empty();
    for item in items {
        if searching && !gz_ui::search::matches(search, item) {
            continue;
        }
        let parts: Vec<&str> = item.split('\\').collect();
        let Some((_leaf, dirs)) = parts.split_last() else { continue };
        // the common prefix with what is open in the stack
        let common =
            stack.iter().zip(dirs.iter()).take_while(|(a, b)| a.rsplit('\\').next() == Some(**b)).count();
        stack.truncate(common);
        let mut visible = stack.iter().all(|d| searching || open.contains(d));
        for d in dirs.iter().skip(common) {
            let path = match stack.last() {
                Some(parent) => format!("{parent}\\{d}"),
                None => (*d).to_owned(),
            };
            if visible {
                out.push(TreeRow { depth: stack.len() as u16, path: path.clone(), folder: true });
            }
            visible = visible && (searching || open.contains(&path));
            stack.push(path);
        }
        if visible {
            out.push(TreeRow { depth: stack.len() as u16, path: item.clone(), folder: false });
        }
    }
    out
}

/// The tree list of a chooser: `\`-separated items as folders and leaves, filtered by
/// `search`, `height` tall (rows outside the view are not laid out; the rows themselves are
/// worked out off the UI thread, so a keystroke in a list of thousands costs the frame nothing).
/// Returns the item clicked; `picked` is drawn selected.
pub fn local_tree_list(
    ui: &mut Ui,
    editor: &Editor,
    id_salt: &str,
    items: &Arc<Vec<String>>,
    search: &str,
    picked: &str,
    height: f32,
) -> Option<String> {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let id = ui.make_persistent_id(("gz-ui-level.tree", id_salt));
    let open_id = id.with("open");
    let open = ctx.data(|d| d.get_temp::<OpenFolders>(open_id)).unwrap_or_default();
    let mut state = ctx
        .data(|d| d.get_temp::<TreeState>(id))
        .unwrap_or_else(|| TreeState { query: Query::new(editor.exec.clone()), shown: Arc::new(Vec::new()) });
    // the list's identity is its revision: a list read again is another Arc
    let rev = gz_base::Rev(Arc::as_ptr(items) as usize as u64);
    let key = (search.to_owned(), open.epoch);
    let (list, s, set) = (items.clone(), search.to_owned(), open.set.clone());
    if let Some(rows) = state.query.get(&key, rev, move || tree_rows(&list, &s, &set)).value() {
        state.shown = rows.clone();
    }
    ctx.data_mut(|d| d.insert_temp(id, state.clone()));
    let rows = state.shown;
    let mut clicked = None;
    let mut toggle: Option<String> = None;
    let row_h = metrics::ROW;
    let w = ui.available_width();
    egui::Frame::new()
        .fill(p.inset)
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(metrics::RADIUS))
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            ui.set_width((w - 10.0).max(1.0));
            egui::ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .max_height((height - 10.0).max(row_h))
                .auto_shrink([false, false])
                .show_rows(ui, row_h, rows.len(), |ui, range| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for row in rows.get(range).unwrap_or(&[]) {
                        let (rect, _) =
                            ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::hover());
                        let resp = ui.interact(rect, id.with(("row", row.path.as_str())), Sense::click());
                        let selected = !row.folder && row.path.eq_ignore_ascii_case(picked);
                        let painter = ui.painter();
                        if selected {
                            painter.rect_filled(rect, CornerRadius::same(3), p.selection_focused);
                        } else if resp.hovered() {
                            painter.rect_filled(rect, CornerRadius::same(3), p.hover);
                        }
                        let x = rect.min.x + 4.0 + f32::from(row.depth) * 12.0;
                        let leaf = row.path.rsplit('\\').next().unwrap_or(&row.path);
                        let (icon, color) = if row.folder {
                            let is_open = !search.trim().is_empty() || open.set.contains(&row.path);
                            (if is_open { Icon::ChevronDown } else { Icon::ChevronRight }, p.text_secondary)
                        } else {
                            (Icon::Dot, p.text_muted)
                        };
                        icons::draw(
                            painter,
                            icon,
                            Rect::from_min_size(pos2(x, rect.center().y - 6.0), Vec2::splat(12.0)),
                            color,
                        );
                        let tx = x + 16.0;
                        let cut = text::paint(
                            painter,
                            &Font::Body.id(),
                            pos2(tx, rect.center().y),
                            Align2::LEFT_CENTER,
                            if row.folder { p.text_secondary } else { p.text },
                            Tx::raw(leaf),
                            Some((rect.max.x - tx - 4.0).max(0.0)),
                            Clip::End,
                        );
                        if cut {
                            audit::truncated(&ctx, leaf, leaf, rect);
                        }
                        if resp.clicked() {
                            if row.folder {
                                toggle = Some(row.path.clone());
                            } else {
                                clicked = Some(row.path.clone());
                            }
                        }
                        if !row.folder {
                            resp.tip(Tx::raw(row.path.as_str()));
                        }
                    }
                });
        });
    if let Some(folder) = toggle
        && search.trim().is_empty()
    {
        let mut open = open;
        if !open.set.remove(&folder) {
            open.set.insert(folder);
        }
        open.epoch += 1;
        ctx.data_mut(|d| d.insert_temp(open_id, open));
    }
    clicked
}

/// A child `Ui` placed at `rect`, laid out top-down.
pub fn child_at(ui: &mut Ui, salt: impl std::hash::Hash + std::fmt::Debug, rect: Rect) -> Ui {
    let mut child = ui.new_child(
        UiBuilder::new().id_salt(salt).max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)),
    );
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    child
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn closed_folders_hide_their_items() {
        let it = items(&["a\\x", "a\\y", "b"]);
        let rows = tree_rows(&it, "", &rustc_hash::FxHashSet::default());
        assert_eq!(rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
        let mut open = rustc_hash::FxHashSet::default();
        open.insert("a".to_owned());
        let rows = tree_rows(&it, "", &open);
        assert_eq!(rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(), vec!["a", "a\\x", "a\\y", "b"]);
    }

    #[test]
    fn a_search_opens_the_folders_of_its_matches() {
        let it = items(&["props\\box_1", "props\\barrel", "trees\\pine"]);
        let rows = tree_rows(&it, "box", &rustc_hash::FxHashSet::default());
        assert_eq!(rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(), vec!["props", "props\\box_1"]);
    }

    /// The eye and the lock of a row in frames of the kit: each answers its click.
    #[test]
    fn the_eye_and_the_lock_answer_their_clicks() {
        use egui::{Event, Modifiers, PointerButton, Pos2, RawInput};
        let ctx = egui::Context::default();
        gz_ui::theme::install(&ctx, gz_ui::Theme::default_theme());
        let eye = Rect::from_min_size(pos2(10.0, 10.0), Vec2::splat(22.0));
        let lock = Rect::from_min_size(pos2(32.0, 10.0), Vec2::splat(22.0));
        let mut frame = |events: Vec<Event>, t: f64| -> (bool, bool) {
            let mut got = (false, false);
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0))),
                time: Some(t),
                events,
                ..RawInput::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                got.0 = local_eye_toggle(ui, Id::unique("eye"), eye, true, "Hide");
                got.1 = local_lock_toggle(ui, Id::unique("lock"), lock, false, false, "Lock");
            });
            out.textures_delta.clear();
            got
        };
        let click = |at: Pos2, frame: &mut dyn FnMut(Vec<Event>, f64) -> (bool, bool), t: f64| {
            let b = |pressed| Event::PointerButton {
                pos: at,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            frame(vec![Event::PointerMoved(at)], t);
            frame(vec![b(true)], t + 0.05);
            frame(vec![b(false)], t + 0.1)
        };
        assert_eq!(click(eye.center(), &mut frame, 0.0), (true, false));
        // an unlocked lock is drawn only under the pointer, and still takes its click
        assert_eq!(click(lock.center(), &mut frame, 1.0), (false, true));
    }
}
