//! The Keyboard page of Preferences: a Command | Shortcut | [Assign...] [Clear] table grouped as
//! the menus are, under the official names (the palette's), with keycaps, over the shell's
//! binding table - read and changed through the `shortcuts` command, as MCP does
//! (`CCustomPreferences::DrawKeyboard`, `UIKeyPressForm`).

use std::sync::Arc;

use egui::{Align, Align2, Layout, Rect, Sense, Stroke, Ui, UiBuilder, Vec2, pos2};
use gz_app::Chord;
use gz_app::keys::{consume, pressed, shell_chord};
use gz_editor::{Editor, Value};
use gz_ui::dialog::Footer;
use gz_ui::list::{Column, table_headers};
use gz_ui::state::busy_line;
use gz_ui::text::{self, Clip};
use gz_ui::units::{button_width, em};
use gz_ui::{Button, Dialog, DialogSize, Font, Kind, Tone, keycaps, keycaps_width, palette};
use parking_lot::Mutex;

use crate::reply::{args, call_then, error_text, ok, s};

/// What separates the parts of a command's name: "Edit · Undo".
pub const PATH_DOT: &str = " \u{b7} ";

/// The commands that take a shortcut in the order the Keyboard page lists them, as the menus hold
/// them (`kCommandNames`); a command missing here comes after them, by its group.
pub const COMMAND_ORDER: &[&str] = &[
    "COMMAND_CLEAR",
    "COMMAND_LOAD",
    "COMMAND_LOAD_FIRSTRECENT",
    "COMMAND_SAVE.\"Save\"",
    "COMMAND_SAVE.\"Save As\"",
    "COMMAND_FILE_MENU",
    "COMMAND_EXIT",
    "COMMAND_UNDO",
    "COMMAND_REDO",
    "COMMAND_CUT",
    "COMMAND_COPY",
    "COMMAND_PASTE",
    "COMMAND_DELETE_SELECTION",
    "COMMAND_SHOW_OBJECTLIST",
    "COMMAND_COMMAND_PALETTE",
    "COMMAND_EDITOR_PREF",
    "COMMAND_BREAK_LAST_OPERATION",
    "COMMAND_SELECT_ALL",
    "COMMAND_DESELECT_ALL",
    "COMMAND_INVERT_SELECTION_ALL",
    "COMMAND_HIDE_SEL",
    "COMMAND_HIDE_UNSEL",
    "COMMAND_HIDE_ALL",
    "COMMAND_GIZMO_MODE.\"Select\"",
    "COMMAND_CHANGE_ACTION.\"Add\"",
    "COMMAND_GIZMO_MODE.\"Move\"",
    "COMMAND_GIZMO_MODE.\"Rotate\"",
    "COMMAND_GIZMO_MODE.\"Scale\"",
    "COMMAND_GIZMO_MODE.\"Cycle\"",
    "COMMAND_CHANGE_ACTION.\"Select\"",
    "COMMAND_ZOOM_EXTENTS.\"Extent\"",
    "COMMAND_ZOOM_EXTENTS.\"Selected\"",
    "COMMAND_MOVE_CAMERA_TO",
    "COMMAND_TOGGLE_RENDER_WIRE",
    "COMMAND_TOGGLE_GRID",
    "COMMAND_GRID_SLOT_SIZE.\"Decrease\"",
    "COMMAND_GRID_SLOT_SIZE.\"Increase\"",
    "COMMAND_TOGGLE_SAFE_RECT",
    "COMMAND_TOGGLE_AIMAP_VISIBILITY",
    "COMMAND_CHANGE_TARGET.\"Object\"",
    "COMMAND_CHANGE_TARGET.\"Light\"",
    "COMMAND_CHANGE_TARGET.\"Sound Source\"",
    "COMMAND_CHANGE_TARGET.\"Sound Env\"",
    "COMMAND_CHANGE_TARGET.\"Glow\"",
    "COMMAND_CHANGE_TARGET.\"Shape\"",
    "COMMAND_CHANGE_TARGET.\"Spawn Point\"",
    "COMMAND_CHANGE_TARGET.\"Way\"",
    "COMMAND_CHANGE_TARGET.\"Way Point\"",
    "COMMAND_CHANGE_TARGET.\"Toggle Way Mode\"",
    "COMMAND_CHANGE_TARGET.\"Sector\"",
    "COMMAND_CHANGE_TARGET.\"Portal\"",
    "COMMAND_CHANGE_TARGET.\"Group\"",
    "COMMAND_CHANGE_TARGET.\"Particle System\"",
    "COMMAND_CHANGE_TARGET.\"Detail Objects\"",
    "COMMAND_CHANGE_TARGET.\"AI Map\"",
    "COMMAND_CHANGE_TARGET.\"Static Wallmark\"",
    "COMMAND_CHANGE_TARGET.\"Fog Volume\"",
    "COMMAND_DROP_TO_GROUND",
    "COMMAND_SIMULATE",
    "COMMAND_USE_SIMULATE_POSITIONS",
    "COMMAND_BUILD",
    "COMMAND_MAKE_GAME",
    "COMMAND_MAKE_DETAILS",
    "COMMAND_MAKE_HOM",
    "COMMAND_MAKE_SOM",
    "COMMAND_MAKE_AIMAP",
    "COMMAND_MOD_BUILD",
    "COMMAND_MOD_EDIT_MANIFEST",
    "COMMAND_MOD_CHARACTERS",
    "COMMAND_MOD_WORLD",
    "COMMAND_MOD_PDA",
    "COMMAND_MOD_PACKAGE",
    "COMMAND_MOD_PUBLISH",
    "COMMAND_MOD_RELEASES",
    "COMMAND_MOD_CATALOG",
    "COMMAND_SHOW_PROPERTIES",
    "COMMAND_OPTIONS",
    "COMMAND_SHOW_WORLD_OUTLINER",
    "COMMAND_SHOW_CONTENT_BROWSER",
    "COMMAND_RUN_MACRO.\"Slot #1\"",
    "COMMAND_RUN_MACRO.\"Slot #2\"",
    "COMMAND_RUN_MACRO.\"Slot #3\"",
    "COMMAND_RUN_MACRO.\"Slot #4\"",
    "COMMAND_RUN_MACRO.\"Slot #5\"",
    "COMMAND_RUN_MACRO.\"Slot #6\"",
    "COMMAND_RUN_MACRO.\"Slot #7\"",
    "COMMAND_RUN_MACRO.\"Slot #8\"",
];

/// A row of the Keyboard page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutRow {
    /// The command's `.ini` name: `COMMAND_UNDO`, `COMMAND_ZOOM_EXTENTS."Selected"`.
    pub id: String,
    pub group: String,
    pub label: String,
    /// What it is bound to now: "Ctrl+Z", or empty.
    pub keys: String,
}

impl ShortcutRow {
    /// "group · label", the name the shell and MCP know it by.
    #[must_use]
    pub fn full_label(&self) -> String {
        if self.group.is_empty() { self.label.clone() } else { format!("{}{PATH_DOT}{}", self.group, self.label) }
    }
}

/// The rows of a `shortcuts action=list` reply, in the page's order: the listed commands as
/// listed, the rest after them by group.
#[must_use]
pub fn parse_rows(reply: &Value) -> Vec<ShortcutRow> {
    let mut rows: Vec<ShortcutRow> = reply
        .get("shortcuts")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|r| {
                    let text_ = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
                    let full = text_("label");
                    let (group, label) = match full.split_once(PATH_DOT) {
                        Some((g, l)) => (g.to_owned(), l.to_owned()),
                        None => ("General".to_owned(), full.clone()),
                    };
                    ShortcutRow { id: text_("id"), group, label, keys: text_("keys") }
                })
                .collect()
        })
        .unwrap_or_default();
    let order = |r: &ShortcutRow| COMMAND_ORDER.iter().position(|id| *id == r.id).unwrap_or(usize::MAX);
    rows.sort_by(|a, b| {
        let (oa, ob) = (order(a), order(b));
        oa.cmp(&ob).then_with(|| if oa == usize::MAX { a.group.cmp(&b.group) } else { std::cmp::Ordering::Equal })
    });
    rows
}

/// A search finds a command by its name, its group, its keys and its id.
#[must_use]
pub fn row_matches(search: &str, row: &ShortcutRow) -> bool {
    gz_ui::search::matches(search, &format!("{} {} {} {}", row.group, row.label, row.keys, row.id))
}

/// The chord a key press is, for a shortcut: Esc and Enter alone answer the dialog instead.
#[must_use]
pub fn takes(chord: &Chord) -> bool {
    let bare = !chord.ctrl && !chord.shift && !chord.alt;
    !(bare && matches!(chord.key, egui::Key::Escape | egui::Key::Enter))
}

/// The Assign dialog: whose shortcut, and the chord pressed so far.
struct Assign {
    id: String,
    name: String,
    chord: Option<Chord>,
}

/// The page's view: the rows as the shell last listed them and the Assign dialog.
#[derive(Default)]
pub struct KeyboardPage {
    rows: Arc<Mutex<Option<Vec<ShortcutRow>>>>,
    assign: Option<Assign>,
}

/// What a frame of the table asked for.
enum Ask {
    Assign(String, String),
    Clear(String),
}

impl KeyboardPage {
    /// Lists the shortcuts again, off the UI thread.
    pub fn reload(&self, editor: &Arc<Editor>) {
        let rows = self.rows.clone();
        call_then(editor, "shortcuts", args([("action", s("list"))]), move |reply, _| {
            if ok(reply) {
                *rows.lock() = Some(parse_rows(reply));
            }
        });
    }

    /// How many commands a search finds.
    #[must_use]
    pub fn matches(&self, search: &str) -> usize {
        self.rows.lock().as_ref().map_or(0, |rows| rows.iter().filter(|r| row_matches(search, r)).count())
    }

    /// The table; with a search, only the commands it finds.
    pub fn draw(&mut self, ui: &mut Ui, editor: &Arc<Editor>, search: &str) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let mut ask = None;
        {
            let guard = self.rows.lock();
            let Some(rows) = guard.as_ref() else {
                busy_line(ui, "Reading the shortcuts...");
                return;
            };
            let shown: Vec<&ShortcutRow> = rows.iter().filter(|r| search.is_empty() || row_matches(search, r)).collect();
            // the Shortcut column is as wide as the widest keycaps
            let keys_w = shown.iter().map(|r| keycaps_width(ui, &r.keys) + 16.0).fold(em(6.0), f32::max);
            let assign_w = button_width(&ctx, "Assign...", Kind::Secondary, false, 0.0);
            let clear_w = button_width(&ctx, "Clear", Kind::Secondary, false, 0.0);
            let actions_w = assign_w + clear_w + 8.0 + 12.0;
            let width = ui.available_width();
            let command_w = (width - keys_w - actions_w).max(em(8.0));
            table_headers(ui, &[Column::new("Command", command_w), Column::new("Shortcut", keys_w), Column::new("", actions_w)]);
            let mut group: Option<&str> = None;
            for row in shown {
                // the group's name over its commands, as the menus hold them
                if group != Some(row.group.as_str()) {
                    group = Some(row.group.as_str());
                    let (r, _) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::hover());
                    text::paint(ui.painter(), &Font::Strong.id(), pos2(r.min.x + 6.0, r.center().y), Align2::LEFT_CENTER, p.text, &row.group, Some(command_w - 12.0), Clip::End);
                }
                let (r, _) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::hover());
                let x = r.min.x + 6.0 + 12.0;
                let cut = text::paint(ui.painter(), &Font::Body.id(), pos2(x, r.center().y), Align2::LEFT_CENTER, p.text, &row.label, Some(command_w - 18.0 - 6.0), Clip::End);
                // the id MCP and the settings file know the command by is its tooltip
                let label_rect = Rect::from_min_max(pos2(x, r.min.y), pos2(r.min.x + command_w, r.max.y));
                let tip = if cut { format!("{}\n{}", row.label, row.id) } else { row.id.clone() };
                ui.interact(label_rect, ui.make_persistent_id(("gz-ui-app.shortcut", row.id.as_str())), Sense::hover()).on_hover_text(tip);
                let keys_x = r.min.x + command_w + 6.0;
                if row.keys.is_empty() {
                    ui.painter().text(pos2(keys_x, r.center().y), Align2::LEFT_CENTER, "None", Font::Body.id(), p.text_muted);
                } else {
                    let w = keycaps_width(ui, &row.keys);
                    keycaps(ui, pos2(keys_x + w, r.center().y), &row.keys, true);
                }
                let cell = Rect::from_min_max(pos2(r.min.x + command_w + keys_w + 6.0, r.min.y), r.max);
                let mut c = ui.new_child(UiBuilder::new().id_salt(("gz-ui-app.shortcut.actions", row.id.as_str())).max_rect(cell).layout(Layout::left_to_right(Align::Center)));
                c.spacing_mut().item_spacing.x = 8.0;
                if Button::new("Assign...").tip("Press the new shortcut in the dialog that opens").show(&mut c).clicked() {
                    ask = Some(Ask::Assign(row.id.clone(), row.full_label()));
                }
                if Button::new("Clear").enabled(!row.keys.is_empty(), "This command has no shortcut").show(&mut c).clicked() {
                    ask = Some(Ask::Clear(row.id.clone()));
                }
                ui.painter().line_segment([pos2(r.min.x, r.max.y - 0.5), pos2(r.max.x, r.max.y - 0.5)], Stroke::new(1.0, p.border_subtle));
            }
        }
        match ask {
            Some(Ask::Assign(id, name)) => self.assign = Some(Assign { id, name, chord: None }),
            Some(Ask::Clear(id)) => {
                let rows = self.rows.clone();
                call_then(editor, "shortcuts", args([("action", s("clear")), ("command", s(&id))]), move |reply, ctx| {
                    if ok(reply) {
                        set_row_keys(&rows, &id, "");
                    } else {
                        gz_ui::state::toast(ctx, Tone::Error, &error_text(reply));
                    }
                });
            }
            None => {}
        }
    }

    /// The Assign dialog, while it is up: the next shortcut pressed, who has it now, Assign.
    pub fn draw_assign(&mut self, ui: &mut Ui, editor: &Arc<Editor>) {
        let Some(a) = self.assign.as_mut() else { return };
        let ctx = ui.ctx().clone();
        // the press of this frame that is a shortcut: taken, so nothing else acts on it
        let chords = ctx.input(|i| pressed(&i.events));
        if let Some(c) = chords.into_iter().rev().find(takes) {
            ctx.input_mut(|i| consume(i, &c));
            a.chord = Some(c);
        }
        let label = a.chord.map(|c| c.label()).unwrap_or_default();
        let owner = {
            let rows = self.rows.lock();
            rows.as_ref().and_then(|rows| rows.iter().find(|r| !label.is_empty() && r.keys == label && r.id != a.id).map(ShortcutRow::full_label))
        };
        let mut open = true;
        let mut assign = false;
        let mut cancel = false;
        let has = a.chord.is_some();
        let reserved = a.chord.is_some_and(|c| shell_chord(&c));
        let name = a.name.clone();
        Dialog::new("Assign Shortcut", "dlg.assign_shortcut", DialogSize::S).show(
            ui,
            &mut open,
            |ui| {
                let p = palette(ui.ctx());
                text::label(ui, &name, Font::Strong, p.text, Clip::Middle);
                text::secondary(ui, "Press the keys of the new shortcut.");
                ui.add_space(8.0);
                let (r, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::hover());
                if label.is_empty() {
                    ui.painter().text(pos2(r.min.x, r.center().y), Align2::LEFT_CENTER, "Waiting for a key...", Font::Body.id(), p.text_muted);
                } else {
                    let w = keycaps_width(ui, &label);
                    keycaps(ui, pos2(r.min.x + w, r.center().y), &label, true);
                }
                if let Some(owner) = &owner {
                    ui.add_space(4.0);
                    text::status(ui, Tone::Warning, &format!("{label} is bound to {owner}: it moves to this command."));
                }
                if reserved {
                    ui.add_space(4.0);
                    text::status(ui, Tone::Warning, "The shell keeps this shortcut for itself: the scene never sees it.");
                }
            },
            |f: &mut Footer<'_>| {
                if f.button("Assign", Kind::Primary, true, has, "Press a shortcut first.") {
                    assign = true;
                }
                if f.button("Cancel", Kind::Secondary, false, true, "") {
                    cancel = true;
                }
            },
        );
        if assign {
            let (id, keys) = (a.id.clone(), label.clone());
            let new_name = a.name.clone();
            let rows = self.rows.clone();
            let ed = editor.clone();
            call_then(editor, "shortcuts", args([("action", s("set")), ("command", s(&id)), ("keys", s(&keys))]), move |reply, ctx| {
                if !ok(reply) {
                    gz_ui::state::toast(ctx, Tone::Error, &error_text(reply));
                    return;
                }
                // one shortcut, one command: the one that had it lets it go
                if let Some(from) = reply.get("moved_from").and_then(Value::as_str) {
                    let old = rows.lock().as_ref().and_then(|r| r.iter().find(|r| r.id == from).map(ShortcutRow::full_label)).unwrap_or_else(|| from.to_owned());
                    ed.log.info(format!("Shortcut {keys} moved from {old} to {new_name}"));
                    set_row_keys(&rows, from, "");
                }
                set_row_keys(&rows, &id, &keys);
            });
            self.assign = None;
        } else if !open || cancel {
            self.assign = None;
        }
    }
}

fn set_row_keys(rows: &Mutex<Option<Vec<ShortcutRow>>>, id: &str, keys: &str) {
    if let Some(row) = rows.lock().as_mut().and_then(|r| r.iter_mut().find(|r| r.id == id)) {
        keys.clone_into(&mut row.keys);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_come_in_the_menus_order() {
        let reply = serde_json::json!({ "ok": true, "shortcuts": [
            { "id": "COMMAND_ZZZ", "label": "Mod · Something", "keys": "" },
            { "id": "COMMAND_REDO", "label": "Edit · Redo", "keys": "Ctrl+Y" },
            { "id": "COMMAND_AAA", "label": "Build · Other", "keys": "" },
            { "id": "COMMAND_UNDO", "label": "Edit · Undo", "keys": "Ctrl+Z" },
        ]});
        let rows = parse_rows(&reply);
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, ["COMMAND_UNDO", "COMMAND_REDO", "COMMAND_AAA", "COMMAND_ZZZ"]);
        assert_eq!(rows.first().map(|r| (r.group.as_str(), r.label.as_str())), Some(("Edit", "Undo")));
        assert_eq!(rows.first().map(ShortcutRow::full_label).as_deref(), Some("Edit \u{b7} Undo"));
    }

    #[test]
    fn a_search_finds_names_keys_and_ids() {
        let row = ShortcutRow { id: "COMMAND_UNDO".to_owned(), group: "Edit".to_owned(), label: "Undo".to_owned(), keys: "Ctrl+Z".to_owned() };
        assert!(row_matches("undo", &row));
        assert!(row_matches("ctrl+z", &row));
        assert!(row_matches("command_undo", &row));
        assert!(!row_matches("redo", &row));
    }

    #[test]
    fn esc_and_enter_answer_the_dialog() {
        assert!(!takes(&Chord::new(egui::Key::Escape)));
        assert!(!takes(&Chord::new(egui::Key::Enter)));
        assert!(takes(&Chord::ctrl(egui::Key::Enter)));
        assert!(takes(&Chord::new(egui::Key::F5)));
    }
}
