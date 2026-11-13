//! The Keyboard page of Preferences: every action of the editor with its keys, grouped as the
//! palette groups them, read and changed through `app.keys` - the command MCP runs - so the page
//! and an agent see and change the same table.
//!
//! The keys of a row are its click target: a click records the next chord ("Press keys..."), Esc
//! gives up, Backspace or the cross that shows on hover takes the keys away. A chord another
//! action of an overlapping scope had is taken from it, and the page says so. A row whose keys are
//! the same as another's says which; one the author changed has its Reset. The rows are laid out
//! only where they are on screen.

use std::sync::Arc;

use egui::{Align2, CornerRadius, Key, Rect, Sense, Stroke, StrokeKind, Ui, Vec2, pos2};
use gz_app::Chord;
use gz_app::keys::{consume, pressed};
use gz_editor::{Editor, Value};
use gz_ui::dialog::{self, Answer, ConfirmSpec};
use gz_ui::list::{Column, table_headers};
use gz_ui::state::busy_line;
use gz_ui::text::{self, Clip};
use gz_ui::units::{button_width, em};
use gz_ui::{Button, Font, Icon, Kind, Tone, icons, keycaps, keycaps_width, palette};
use parking_lot::Mutex;

use crate::reply::{args, call_then, error_text, ok, s};

/// A row of the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyRow {
    /// The action's id (`edit.undo`), or a COMMAND_* id of the level editor's own table.
    pub id: String,
    pub title: String,
    pub group: String,
    /// What it is bound to now, and by default.
    pub keys: Vec<String>,
    pub default: Vec<String>,
    /// The other actions with one of its keys where both are live.
    pub conflicts: Vec<String>,
}

impl KeyRow {
    /// The author bound it to other keys than its own.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.keys != self.default
    }
}

/// The rows of an `app.keys action=list` reply, grouped: the groups in the order they first come,
/// the rows of a group in theirs.
#[must_use]
pub fn parse_rows(reply: &Value) -> Vec<KeyRow> {
    let strings = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
            .unwrap_or_default()
    };
    let rows: Vec<KeyRow> = reply
        .get("keys")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|r| {
                    let text_ = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
                    KeyRow {
                        id: text_("id"),
                        title: text_("title").trim_end_matches("...").to_owned(),
                        group: text_("group"),
                        keys: strings(r.get("keys")),
                        default: strings(r.get("default")),
                        conflicts: strings(r.get("conflicts")),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let mut groups: Vec<String> = Vec::new();
    for r in &rows {
        if !groups.contains(&r.group) {
            groups.push(r.group.clone());
        }
    }
    let mut out = Vec::with_capacity(rows.len());
    for g in &groups {
        out.extend(rows.iter().filter(|r| &r.group == g).cloned());
    }
    out
}

/// A search finds a row by its title, its group, its keys and its id.
#[must_use]
pub fn row_matches(search: &str, row: &KeyRow) -> bool {
    gz_ui::search::matches(search, &format!("{} {} {} {}", row.group, row.title, row.keys.join(" "), row.id))
}

/// A chord the page takes for a shortcut: Esc gives up, Enter alone is no shortcut.
#[must_use]
pub fn takes(chord: &Chord) -> bool {
    let bare = !chord.ctrl && !chord.shift && !chord.alt;
    !(bare && matches!(chord.key, Key::Escape | Key::Enter))
}

/// A line of the table: a group's heading or a row.
#[derive(Clone, Copy, Debug)]
enum Line {
    Group(usize),
    Row(usize),
}

/// The page's view: the rows as `app.keys` last listed them, the row recording a chord, the
/// last note.
#[derive(Default)]
pub struct KeyboardPage {
    rows: Arc<Mutex<Option<Vec<KeyRow>>>>,
    recording: Option<String>,
    note: Arc<Mutex<Option<(Tone, String)>>>,
}

const ROW_H: f32 = 30.0;

impl KeyboardPage {
    /// Lists the keys again, off the UI thread.
    pub fn reload(&self, editor: &Arc<Editor>) {
        let rows = self.rows.clone();
        call_then(editor, "app.keys", args([("action", s("list"))]), move |reply, _| {
            if ok(reply) {
                *rows.lock() = Some(parse_rows(reply));
            }
        });
    }

    /// How many rows a search finds.
    #[must_use]
    pub fn matches(&self, search: &str) -> usize {
        self.rows.lock().as_ref().map_or(0, |rows| rows.iter().filter(|r| row_matches(search, r)).count())
    }

    /// Runs `app.keys` and lists again; a key taken from another action is said.
    fn change(&self, editor: &Arc<Editor>, a: gz_editor::Args, title: String) {
        let (rows, note) = (self.rows.clone(), self.note.clone());
        let ed = editor.clone();
        call_then(editor, "app.keys", a, move |reply, _| {
            if !ok(reply) {
                *note.lock() = Some((Tone::Error, error_text(reply)));
                return;
            }
            let moved: Vec<String> = reply
                .get("moved_from")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default();
            let names: Vec<String> = rows
                .lock()
                .as_ref()
                .map(|r| r.iter().filter(|x| moved.contains(&x.id)).map(|x| x.title.clone()).collect())
                .unwrap_or_default();
            *note.lock() = (!names.is_empty())
                .then(|| (Tone::Warning, format!("{title} took its keys from {}.", names.join(", "))));
            call_then(&ed, "app.keys", args([("action", s("list"))]), move |reply, _| {
                if ok(reply) {
                    *rows.lock() = Some(parse_rows(reply));
                }
            });
        });
    }

    /// The table; with a search, only the rows it finds.
    pub fn draw(&mut self, ui: &mut Ui, editor: &Arc<Editor>, search: &str) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let Some(rows) = self.rows.lock().clone() else {
            busy_line(ui, "Reading the keys...");
            return;
        };
        let shown: Vec<usize> =
            (0..rows.len()).filter(|&i| search.is_empty() || row_matches(search, &rows[i])).collect();
        let mut lines: Vec<Line> = Vec::with_capacity(shown.len() + 16);
        let mut last_group: Option<&str> = None;
        for &i in &shown {
            if last_group != Some(rows[i].group.as_str()) {
                last_group = Some(rows[i].group.as_str());
                lines.push(Line::Group(i));
            }
            lines.push(Line::Row(i));
        }

        // above the table: the note of the last change, and Reset All
        let changed = rows.iter().filter(|r| r.changed()).count();
        ui.horizontal(|ui| {
            let note = self.note.lock().clone();
            match note {
                Some((tone, text_)) => {
                    let color = p.tone(tone);
                    text::label(ui, &text_, Font::Body, color, Clip::End);
                }
                None => {
                    let muted = p.text_secondary;
                    text::label(
                        ui,
                        "Click a shortcut to change it; Backspace takes it away.",
                        Font::Body,
                        muted,
                        Clip::End,
                    );
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = "Reset All";
                if ui
                    .add(
                        Button::new(label)
                            .kind(Kind::GhostDanger)
                            .enabled(changed > 0, "Every shortcut is its default."),
                    )
                    .clicked()
                {
                    self.ask_reset_all(&ctx, editor, changed);
                }
            });
        });
        ui.add_space(4.0);

        let width = ui.available_width();
        let keys_w = em(12.0);
        let reset_w = button_width(&ctx, "Reset", Kind::Ghost, false, 0.0) + 8.0;
        let title_w = (width - keys_w - reset_w).max(em(10.0));
        table_headers(
            ui,
            &[Column::new("Action", title_w), Column::new("Keys", keys_w), Column::new("", reset_w)],
        );

        // the row recording: the keys of the frame go to it, and to nothing else
        if let Some(id) = self.recording.clone() {
            let presses = ctx.input(|i| pressed(&i.events));
            for chord in &presses {
                ctx.input_mut(|i| consume(i, chord));
            }
            if let Some(chord) = presses.first().copied() {
                if chord.key == Key::Escape && !chord.ctrl && !chord.shift && !chord.alt {
                    self.recording = None;
                } else if chord.key == Key::Backspace && !chord.ctrl && !chord.shift && !chord.alt {
                    self.recording = None;
                    let title = rows.iter().find(|r| r.id == id).map(|r| r.title.clone()).unwrap_or_default();
                    self.change(editor, args([("action", s("clear")), ("id", s(&id))]), title);
                } else if takes(&chord) {
                    self.recording = None;
                    let title = rows.iter().find(|r| r.id == id).map(|r| r.title.clone()).unwrap_or_default();
                    self.change(
                        editor,
                        args([("action", s("bind")), ("id", s(&id)), ("keys", s(&chord.label()))]),
                        title,
                    );
                }
            }
        }

        let mut record = None;
        let mut clear = None;
        let mut reset = None;
        egui::ScrollArea::vertical().id_salt("gz-ui-app.keyboard").auto_shrink([false, false]).show_rows(
            ui,
            ROW_H,
            lines.len(),
            |ui, range| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for n in range {
                    let Some(line) = lines.get(n).copied() else { continue };
                    let (r, _) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::hover());
                    match line {
                        Line::Group(i) => {
                            ui.painter().text(
                                pos2(r.min.x + 6.0, r.center().y + 3.0),
                                Align2::LEFT_CENTER,
                                &rows[i].group,
                                Font::Strong.id(),
                                p.text,
                            );
                        }
                        Line::Row(i) => {
                            let row = &rows[i];
                            let hover = ui.rect_contains_pointer(r);
                            if hover {
                                ui.painter().rect_filled(r, CornerRadius::same(4), p.hover);
                            }
                            // the title, and what it shares its keys with
                            let title_room = title_w - 18.0;
                            text::paint(
                                ui.painter(),
                                &Font::Body.id(),
                                pos2(r.min.x + 18.0, r.center().y),
                                Align2::LEFT_CENTER,
                                p.text,
                                &row.title,
                                Some(title_room),
                                Clip::End,
                            );
                            let cell = Rect::from_min_size(
                                pos2(r.min.x + title_w, r.min.y + 2.0),
                                Vec2::new(keys_w - 8.0, ROW_H - 4.0),
                            );
                            let resp = ui.interact(
                                cell,
                                ui.make_persistent_id(("gz-ui-app.key", &row.id)),
                                Sense::click(),
                            );
                            let recording = self.recording.as_deref() == Some(row.id.as_str());
                            let outline = if recording {
                                p.accent_bright
                            } else if resp.hovered() {
                                p.border
                            } else {
                                p.border_subtle
                            };
                            ui.painter().rect(
                                cell,
                                CornerRadius::same(4),
                                if recording || resp.hovered() { p.input_hover } else { p.input },
                                Stroke::new(if recording { 2.0 } else { 1.0 }, outline),
                                StrokeKind::Inside,
                            );
                            if recording {
                                ui.painter().text(
                                    pos2(cell.min.x + 8.0, cell.center().y),
                                    Align2::LEFT_CENTER,
                                    "Press keys...",
                                    Font::Body.id(),
                                    p.accent_bright,
                                );
                                ui.ctx().request_repaint();
                            } else if row.keys.is_empty() {
                                ui.painter().text(
                                    pos2(cell.min.x + 8.0, cell.center().y),
                                    Align2::LEFT_CENTER,
                                    "None",
                                    Font::Body.id(),
                                    p.text_muted,
                                );
                            } else {
                                let first = row.keys.first().cloned().unwrap_or_default();
                                let w = keycaps_width(ui, &first);
                                keycaps(ui, pos2(cell.min.x + 8.0 + w, cell.center().y), &first, true);
                                if row.keys.len() > 1 {
                                    ui.painter().text(
                                        pos2(cell.min.x + 14.0 + w, cell.center().y),
                                        Align2::LEFT_CENTER,
                                        format!("+{}", row.keys.len() - 1),
                                        Font::Small.id(),
                                        p.text_muted,
                                    );
                                }
                            }
                            // the hover cross takes the keys away
                            if !row.keys.is_empty() && !recording && (resp.hovered() || hover) {
                                let x = Rect::from_center_size(
                                    pos2(cell.max.x - 12.0, cell.center().y),
                                    Vec2::splat(18.0),
                                );
                                let xr = ui.interact(
                                    x,
                                    ui.make_persistent_id(("gz-ui-app.key.clear", &row.id)),
                                    Sense::click(),
                                );
                                icons::draw(
                                    ui.painter(),
                                    Icon::Close,
                                    x.shrink(3.0),
                                    if xr.hovered() { p.text } else { p.text_secondary },
                                );
                                if xr.on_hover_text("Take the keys away").clicked() {
                                    clear = Some(i);
                                }
                            }
                            let tip = if row.conflicts.is_empty() {
                                format!("{}\nClick, then press the keys.", row.id)
                            } else {
                                let names: Vec<&str> = rows
                                    .iter()
                                    .filter(|x| row.conflicts.contains(&x.id))
                                    .map(|x| x.title.as_str())
                                    .collect();
                                format!(
                                    "The same keys as {} where both are live.\n{}",
                                    names.join(", "),
                                    row.id
                                )
                            };
                            gz_ui::audit::control(ui.ctx(), &row.title, true, None, Some(&tip), cell);
                            if resp.on_hover_text(tip).clicked() && clear.is_none() {
                                record = Some(row.id.clone());
                            }
                            if !row.conflicts.is_empty() {
                                icons::draw(
                                    ui.painter(),
                                    Icon::Warning,
                                    Rect::from_min_size(
                                        pos2(r.min.x + 2.0, r.center().y - 6.0),
                                        Vec2::splat(12.0),
                                    ),
                                    p.warning,
                                );
                            }
                            if row.changed() {
                                let b = Rect::from_min_size(
                                    pos2(r.max.x - reset_w + 4.0, r.min.y + 3.0),
                                    Vec2::new(reset_w - 8.0, ROW_H - 6.0),
                                );
                                if ui
                                    .put(
                                        b,
                                        Button::new("Reset").kind(Kind::Ghost).tip("Back to its own keys"),
                                    )
                                    .clicked()
                                {
                                    reset = Some(i);
                                }
                            }
                        }
                    }
                }
            },
        );
        if let Some(id) = record {
            self.recording = Some(id);
            *self.note.lock() = None;
        }
        if let Some(i) = clear {
            let row = &rows[i];
            self.change(editor, args([("action", s("clear")), ("id", s(&row.id))]), row.title.clone());
        }
        if let Some(i) = reset {
            let row = &rows[i];
            self.change(editor, args([("action", s("reset")), ("id", s(&row.id))]), row.title.clone());
        }
    }

    /// Whether a row records a chord: the shell leaves the keys to the page meanwhile.
    #[must_use]
    pub fn recording(&self) -> bool {
        self.recording.is_some()
    }

    fn ask_reset_all(&self, ctx: &egui::Context, editor: &Arc<Editor>, changed: usize) {
        let spec = ConfirmSpec {
            id: "reset_keys".to_owned(),
            title: "Reset All Keys".to_owned(),
            headline: format!("Put {changed} changed shortcut(s) back to their defaults?"),
            items: Vec::new(),
            total: 0,
            consequence: "The keys you bound are forgotten; every action has its own keys again.".to_owned(),
            action: "Reset All".to_owned(),
            destructive: false,
        };
        let asker = if editor.flags.unattended { gz_ui::Asker::Unattended } else { gz_ui::Asker::Ui };
        let (rows, ed) = (self.rows.clone(), editor.clone());
        let line = dialog::confirm(ctx, asker, spec, move |answer| {
            if answer != Answer::Yes {
                return;
            }
            let ed2 = ed.clone();
            call_then(&ed, "app.keys", args([("action", s("reset"))]), move |_, _| {
                call_then(&ed2, "app.keys", args([("action", s("list"))]), move |reply, _| {
                    if ok(reply) {
                        *rows.lock() = Some(parse_rows(reply));
                    }
                });
            });
        });
        if let Some(line) = line {
            editor.log.plain(line);
        }
    }

    /// Kept for the page's frame: nothing to draw outside the table now.
    pub fn draw_assign(&mut self, _ui: &mut Ui, _editor: &Arc<Editor>) {}
}

#[cfg(test)]
mod tests {
    use super::{KeyRow, parse_rows, row_matches, takes};
    use egui::Key;
    use gz_app::Chord;
    use serde_json::json;

    #[test]
    fn rows_come_grouped_and_say_what_changed() {
        let reply = json!({ "ok": true, "keys": [
            { "id": "file.save", "title": "Save", "group": "File", "keys": ["Ctrl+S"], "default": ["Ctrl+S"], "conflicts": [] },
            { "id": "edit.undo", "title": "Undo", "group": "Edit", "keys": ["Ctrl+U"], "default": ["Ctrl+Z"], "conflicts": [] },
            { "id": "app.quit", "title": "Quit", "group": "File", "keys": [], "default": [], "conflicts": [] },
        ]});
        let rows = parse_rows(&reply);
        assert_eq!(
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            ["file.save", "app.quit", "edit.undo"]
        );
        assert!(rows[2].changed());
        assert!(!rows[0].changed());
        let r = KeyRow {
            id: "graph.frame_all".into(),
            title: "Frame All".into(),
            group: "Graph".into(),
            keys: vec!["Home".into()],
            default: vec![],
            conflicts: vec![],
        };
        assert!(row_matches("graph home", &r));
        assert!(!takes(&Chord::new(Key::Escape)));
        assert!(takes(&Chord::ctrl(Key::Enter)));
    }
}
