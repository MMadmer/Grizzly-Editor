//! The drawing of the Characters tab: the list and the form (`DrawList`, `DrawForm` and their
//! parts in `EditorCharacters.cpp`).

use egui::{Align, Align2, Layout, Rect, Ui, UiBuilder, Vec2, pos2};
use gz_characters::{Character, Supply, Trade, TradeRow, game_id, text_in, text_raw};
use gz_ui::grid::{Grid, integer, number, switch, text_block};
use gz_ui::list::{Column, list, table_headers};
use gz_ui::text::{self, Clip};
use gz_ui::units::{button_width, em};
use gz_ui::{Button, Font, Icon, Kind, PropFlags, PropertyGrid, SearchField, Tone, icon_button, icons, metrics, palette};

use crate::characters::{Action, CharactersDoc, field_of, problem_text, problem_tone};
use crate::lists::{Indexed, ListsView};
use crate::local::{local_cell, local_check, local_dirty_dot, local_hint_field, local_number_text};
use crate::picker::{item_picker, local_combo, local_list_picker, local_multi_combo};
use crate::portraits::{draw_portrait, local_portrait_gallery};

const ROLE: &str = "character";
const LABEL_FRAC: f32 = 0.30;
/// A table row: a field and a little air.
const TABLE_ROW: f32 = 26.0;

/// A row's flags with Invalid when one of its problems is an error (`RowFlags`).
fn flags(own: &[String], field: &str, base: PropFlags) -> PropFlags {
    if own.iter().any(|p| p.starts_with('E') && field_of(p) == field) { base | PropFlags::INVALID } else { base }
}

/// The sentences of a row's problems under it (`FieldMessages`).
fn messages(g: &mut Grid<'_>, own: &[String], field: &str) {
    for p in own.iter().filter(|p| field_of(p) == field) {
        g.message(problem_tone(p), &problem_text(p));
    }
}

/// The same, as lines of their own outside the grid (`FieldStatus`).
fn status_lines(ui: &mut Ui, own: &[String], field: &str) {
    for p in own.iter().filter(|p| field_of(p) == field) {
        let _ = text::status(ui, problem_tone(p), &problem_text(p));
    }
}

/// A cell of a table row: `w` wide, a field high, its content laid out left to right.
fn cell<R>(ui: &mut Ui, w: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    local_cell(ui, Vec2::new(w.max(1.0), metrics::FRAME), Layout::left_to_right(Align::Center), add).inner
}

/// The line under a table row (`BordersInnerH`).
fn row_rule(ui: &mut Ui) {
    let p = palette(ui.ctx());
    let r = ui.min_rect();
    let y = ui.cursor().min.y - 1.0;
    ui.painter().line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], egui::Stroke::new(1.0, p.border_subtle));
}

/// "Caption  (section)" of an item, the section alone when the game names none.
fn item_label(items: &Indexed, section: &str) -> String {
    items.get(section).map_or_else(|| section.to_owned(), Indexed::label)
}

/// "sells ..., a stock of N item(s), asks .. and pays .. of the cost" (`TradeSummary`).
fn trade_summary(t: &Trade) -> String {
    let sells = if t.sells.is_empty() { "its stock only".to_owned() } else { t.sells.join(", ") };
    format!(
        "sells {sells}, a stock of {} item(s), asks {} and pays {} of the cost",
        t.stock.len(),
        gz_luadata::format_g(f64::from(t.markup), 6),
        gz_luadata::format_g(f64::from(t.pays), 6)
    )
}

impl CharactersDoc {
    // ---- the list -------------------------------------------------------------------------------

    pub(crate) fn draw_list(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        ui.horizontal(|ui| {
            let w = (ui.available_width() - metrics::ICON_BUTTON - 6.0).max(em(4.0));
            let _ = SearchField::new(&mut self.list_filter).hint("Search characters").width(w).id_salt("gz-ui-world.characters.search").show(ui);
            if icon_button(ui, Icon::Plus, "Add a character", false, true).clicked() {
                self.actions.push(Action::New);
            }
        });
        ui.add_space(4.0);
        let Some(roster) = self.roster.clone() else {
            gz_ui::state::busy_line(ui, "Reading the characters...");
            return;
        };
        if self.shown.0 != self.list_filter || self.shown.1 != roster.stamp || self.shown.2.len() > roster.raw.len() {
            let rows = roster
                .raw
                .iter()
                .enumerate()
                .filter(|(_, c)| gz_ui::search::matches(&self.list_filter, &format!("{} {} {}", c.id, text_in(&c.name, "rus"), text_in(&c.name, "eng"))))
                .map(|(i, _)| i)
                .collect();
            self.shown = (self.list_filter.clone(), roster.stamp, rows);
        }
        let unread: Vec<&String> = roster.problems.iter().filter(|x| x.starts_with("E100") || x.starts_with("E101")).collect();
        let foot = if unread.is_empty() { 0.0 } else { Font::Body.line() + 8.0 };
        let fresh = self.sel.is_none() && self.dirty;
        let (sel, dirty) = (self.sel, self.dirty);
        let edit_id = self.edit.id.clone();
        let mut new_clicked = false;
        let mut picked: Option<String> = None;
        let shown = &self.shown.2;
        let filtering = !self.list_filter.trim().is_empty();
        list(ui, "gz-ui-world.characters.list", Vec2::new(0.0, -foot), |l| {
            // a new character nobody saved yet heads the list
            if fresh {
                let label = if edit_id.is_empty() { "New character".to_owned() } else { edit_id.clone() };
                let resp = l.row("##new", true, |ui| {
                    let r = ui.max_rect();
                    let x = r.min.x;
                    icons::draw(ui.painter(), Icon::Character, Rect::from_min_size(pos2(x, r.center().y - 7.0), Vec2::splat(metrics::GLYPH_ROW)), p.accent_bright);
                    let tx = x + 20.0;
                    text::paint(ui.painter(), &Font::Body.id(), pos2(tx, r.center().y), Align2::LEFT_CENTER, p.text, &label, Some((r.max.x - tx - 18.0).max(0.0)), Clip::End);
                    local_dirty_dot(ui.painter(), pos2(r.max.x - 10.0, r.center().y), ui.ctx());
                });
                let _ = resp.on_hover_text("Not saved yet");
            }
            for &i in shown {
                let Some(c) = roster.raw.get(i) else { continue };
                let selected = sel == Some(i);
                let name = text_in(&c.name, "rus");
                let resp = l.row(&c.file, selected, |ui| {
                    let r = ui.max_rect();
                    let cy = r.center().y;
                    let icon = if c.is_abstract { Icon::CharacterTemplate } else { Icon::Character };
                    icons::draw(ui.painter(), icon, Rect::from_min_size(pos2(r.min.x, cy - 7.0), Vec2::splat(metrics::GLYPH_ROW)), if selected { p.accent_bright } else { p.text_secondary });
                    // what sits at the right end: the template mark, the dot of unsaved edits
                    let right = if c.is_abstract { em(4.5) } else { 0.0 } + if selected && dirty { 16.0 } else { 0.0 } + 6.0;
                    let x = r.min.x + 20.0;
                    let room = (r.max.x - x - right).max(0.0);
                    let font = Font::Body.id();
                    let id_w = text::width(ui.ctx(), &font, &c.id).min(room);
                    text::paint(ui.painter(), &font, pos2(x, cy), Align2::LEFT_CENTER, p.text, &c.id, Some(room), Clip::End);
                    if !name.is_empty() && room - id_w > em(3.0) {
                        let nx = x + id_w + 8.0;
                        text::paint(ui.painter(), &font, pos2(nx, cy), Align2::LEFT_CENTER, p.text_muted, &name, Some((room - id_w - 8.0).max(0.0)), Clip::End);
                    }
                    if c.is_abstract {
                        let badge = Rect::from_min_max(pos2(r.max.x - right + 2.0, r.min.y), pos2(r.max.x - if selected && dirty { 16.0 } else { 0.0 }, r.max.y));
                        let mut child = ui.new_child(UiBuilder::new().id_salt(("gz-ui-world.template", i)).max_rect(badge).layout(Layout::left_to_right(Align::Center)));
                        gz_ui::state::badge(&mut child, "Template", Tone::Muted);
                    }
                    if selected && dirty {
                        local_dirty_dot(ui.painter(), pos2(r.max.x - 10.0, cy), ui.ctx());
                    }
                });
                let tip = if name.is_empty() { c.file.clone() } else { format!("{}\n{name}", c.file) };
                if resp.on_hover_text(tip).clicked() && !selected {
                    picked = Some(c.file.clone());
                }
            }
            if roster.raw.is_empty() && !fresh {
                if gz_ui::state::empty_state(l.ui(), Icon::Character, "No characters", "The module's own NPCs live here.", Some("New Character")) {
                    new_clicked = true;
                }
            } else if shown.is_empty() && filtering {
                let _ = text::status(l.ui(), Tone::Muted, "No character matches.");
            }
        });
        if !unread.is_empty() {
            let count = if unread.len() == 1 { "1 file does not read".to_owned() } else { format!("{} files do not read", unread.len()) };
            let all: Vec<&str> = unread.iter().map(|x| x.as_str()).collect();
            let _ = text::status(ui, Tone::Error, &count).on_hover_text(all.join("\n"));
        }
        if new_clicked {
            self.actions.push(Action::New);
        }
        if let Some(file) = picked {
            self.actions.push(Action::Select(file));
        }
    }

    // ---- the form -------------------------------------------------------------------------------

    /// The portrait, the name the player reads, the game's name for it, the file; Delete... at the
    /// right; the problems that belong to no field (`DrawHeader`).
    fn draw_header(&mut self, ui: &mut Ui, width: f32) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let portrait = 64.0;
        let delete_w = button_width(&ctx, "Delete...", Kind::GhostDanger, false, 0.0);
        let text_w = (width - portrait * 1.42 - 12.0 - delete_w - 8.0).max(em(4.0));
        let mut title = if self.name_eng.is_empty() { self.name_rus.clone() } else { self.name_eng.clone() };
        if title.is_empty() {
            title = if self.edit.id.is_empty() { "New character".to_owned() } else { self.edit.id.clone() };
        }
        let in_game = if self.module_id.is_empty() {
            "The module has no id yet: Mod Settings".to_owned()
        } else {
            format!("In game: {}", game_id(&self.module_id, &self.edit.id))
        };
        let file = if self.edit.file.is_empty() { "Not saved yet".to_owned() } else { self.edit.file.clone() };
        let mut delete = false;
        ui.horizontal_top(|ui| {
            let _ = draw_portrait(ui, &self.editor, self.sheets.as_ref(), &self.lists, &self.edit.icon, portrait);
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.set_width(text_w);
                let _ = text::label(ui, &title, Font::Title, p.text, Clip::End);
                let _ = text::label(ui, &in_game, Font::Body, p.text_secondary, Clip::End)
                    .on_hover_text("The spawn section, the squad section (with _squad) and the profile a placed stalker carries.");
                let _ = text::label(ui, &file, Font::Body, p.text_secondary, Clip::Start);
                if self.edit.is_abstract {
                    gz_ui::state::badge(ui, "Template", Tone::Muted);
                }
            });
            let used = ui.min_rect().width();
            ui.add_space((width - used - delete_w).max(0.0));
            if Button::new("Delete...").kind(Kind::GhostDanger).enabled(!self.edit.file.is_empty(), "Not saved yet: nothing to delete").show(ui).clicked() {
                delete = true;
            }
        });
        if delete {
            self.actions.push(Action::Delete);
        }
        for problem in self.own.iter().filter(|x| field_of(x).is_empty()) {
            let _ = text::status(ui, problem_tone(problem), &problem_text(problem));
        }
        if !self.status.is_empty() {
            let _ = text::status(ui, Tone::Error, &self.status);
        }
        ui.add_space(6.0);
    }

    pub(crate) fn draw_form(&mut self, ui: &mut Ui, width: f32) {
        self.draw_header(ui, width);
        let module = if self.module_id.is_empty() { "<module>".to_owned() } else { self.module_id.clone() };
        let roster = self.roster.clone();
        let mut changed = false;
        let mut base_changed = false;
        let mut preview = false;
        let editor = self.editor.clone();
        let Self { edit, name_rus, name_eng, bio_rus, bio_eng, inherited, own, lists, sheets, sel, .. } = self;
        let own: &[String] = own;
        let lists: &ListsView = lists;
        let inherited: Option<&Character> = inherited.as_ref();
        let base = edit.base.clone();
        PropertyGrid::new(ROLE).label_frac(LABEL_FRAC).show(ui, |g| {
            g.section("Identity", true, None, |g| {
                let help = format!("a-z, 0-9 and _. The game knows the character as {module}_<id>.");
                if g.row("ID", &help, flags(own, "id", PropFlags::REQUIRED), 0, |ui| local_hint_field(ui, "character.id", &mut edit.id, "").changed()) == Some(true) {
                    changed = true;
                }
                messages(g, own, "id");
                // another character of the project to start from: a base that does not exist is E111
                let others: Vec<(String, bool)> = roster.as_ref().map_or_else(Vec::new, |r| {
                    r.raw.iter().enumerate().filter(|(i, _)| Some(*i) != *sel).map(|(_, c)| (c.id.clone(), c.is_abstract)).collect()
                });
                let known = edit.base.is_empty() || others.iter().any(|(id, _)| *id == edit.base);
                let mut rows = vec!["None".to_owned()];
                rows.extend(others.iter().map(|(id, template)| if *template { format!("{id}  (template)") } else { id.clone() }));
                let current = if edit.base.is_empty() { Some(0) } else { others.iter().position(|(id, _)| *id == edit.base).map(|i| i + 1) };
                let preview_text = if edit.base.is_empty() { "None".to_owned() } else { edit.base.clone() };
                let picked = g.row(
                    "Base",
                    "Another character to start from: what this one does not set comes from there, and the starting kits are merged. None: the character stands on its own.",
                    flags(own, "base", PropFlags::NONE),
                    0,
                    |ui| local_combo(ui, "character.base", &preview_text, !known, &rows, current),
                );
                if let Some(Some(i)) = picked {
                    let next = if i == 0 { String::new() } else { others.get(i - 1).map(|(id, _)| id.clone()).unwrap_or_default() };
                    if next != edit.base {
                        edit.base = next;
                        changed = true;
                        base_changed = true;
                    }
                }
                messages(g, own, "base");
                let flipped = g.row(
                    "Template",
                    "A template only: never a character in the game, so it needs no name and no model. It holds what several characters share.",
                    PropFlags::INLINE,
                    0,
                    |ui| switch(ui, &mut edit.is_abstract).changed(),
                );
                if flipped == Some(true) {
                    changed = true;
                }
            });
            // both languages on screen at once: what a character carries is compared side by side
            g.section("Name and Bio", true, None, |g| {
                let hint = |lang: &str| inherited.map(|b| text_raw(&b.name, lang)).unwrap_or_default();
                let (hint_rus, hint_eng) = (hint("rus"), hint("eng"));
                if g.row("Name (rus)", "What the player reads above the NPC and in dialog.", flags(own, "name", PropFlags::NONE), 0, |ui| {
                    local_hint_field(ui, "character.name_rus", name_rus, &hint_rus).changed()
                }) == Some(true)
                {
                    changed = true;
                }
                messages(g, own, "name");
                if g.row("Name (eng)", "Left empty, an English install reads the Russian name.", PropFlags::NONE, 0, |ui| {
                    local_hint_field(ui, "character.name_eng", name_eng, &hint_eng).changed()
                }) == Some(true)
                {
                    changed = true;
                }
                g.block(|ui| {
                    if text_block(ui, "Bio (rus)", "The line the PDA shows about the character.", bio_rus, 2).changed() {
                        changed = true;
                    }
                    if text_block(ui, "Bio (eng)", "Left empty, an English install reads the Russian bio.", bio_eng, 2).changed() {
                        changed = true;
                    }
                    status_lines(ui, own, "text");
                });
            });
            g.section("Appearance", true, None, |g| {
                let none = match inherited.filter(|b| !b.visual.is_empty()) {
                    Some(b) => format!("From {base}: {}", b.visual),
                    None if !base.is_empty() => "From the base".to_owned(),
                    None => "Not set".to_owned(),
                };
                let help = "The model the NPC spawns with and keeps: meshes\\actors of the linked game and the project. An outfit changes its protection, never its model.";
                if g.row("Model", help, flags(own, "visual", PropFlags::NONE), 0, |ui| {
                    local_list_picker(ui, "character.visual", &mut edit.visual, &lists.visuals, &lists.visuals, Some(&none), !lists.loaded)
                }) == Some(true)
                {
                    changed = true;
                }
                if g.trailing(Icon::Eye, if edit.visual.is_empty() { "Pick a model first" } else { "Preview the model" }) && !edit.visual.is_empty() {
                    preview = true;
                }
                messages(g, own, "visual");
                let help = "The face the dialog window and the PDA show. The gallery holds every portrait of the linked game and the project.";
                if g.row("Portrait", help, flags(own, "icon", PropFlags::NONE), 0, |ui| local_portrait_gallery(ui, "character.icon", &mut edit.icon, &editor, sheets.as_ref(), lists))
                    == Some(true)
                {
                    changed = true;
                }
                messages(g, own, "icon");
            });
            g.section("Profile", true, None, |g| {
                let unset = if base.is_empty() {
                    "Not set: stalker".to_owned()
                } else {
                    match inherited.map(|b| b.community.as_str()).filter(|c| !c.is_empty()) {
                        Some(c) => format!("From {base}: {c}"),
                        None => format!("From {base}"),
                    }
                };
                if g.row("Community", "The communities of the linked game. Not set: the base's, or stalker without one.", flags(own, "community", PropFlags::NONE), 0, |ui| {
                    if lists.loaded && lists.communities.is_empty() {
                        // no linked game to read the list from, so the field stays a field
                        local_hint_field(ui, "character.community", &mut edit.community, "stalker").on_hover_text("No linked game to list the communities of: type the id.").changed()
                    } else {
                        local_list_picker(ui, "character.community", &mut edit.community, &lists.communities, &lists.stalker_communities, Some(&unset), !lists.loaded)
                    }
                }) == Some(true)
                {
                    changed = true;
                }
                messages(g, own, "community");
                if g.row("Rank", "The linked game's ranks by name; the number beside it is the exact value.", PropFlags::NONE, 0, |ui| rank_row(ui, edit, lists)) == Some(true) {
                    changed = true;
                }
                if g.row(
                    "Money",
                    "What the NPC carries when it spawns: a random amount from the first number to the second. Empty: the base's money, or none.",
                    flags(own, "money", PropFlags::NONE),
                    0,
                    |ui| money_row(ui, edit, inherited),
                ) == Some(true)
                {
                    changed = true;
                }
                if edit.has_money && g.trailing(Icon::Clear, if base.is_empty() { "Clear: the NPC spawns without money" } else { "Clear: the base's money applies" }) {
                    edit.has_money = false;
                    edit.money_min = 0;
                    edit.money_max = 0;
                    edit.money_infinite = false;
                    changed = true;
                }
                messages(g, own, "money");
                let flipped = g.row(
                    "Squad",
                    "Also a one-man squad section, so a quest can create the character with spawn.squad as well as with spawn.npc.",
                    PropFlags::INLINE,
                    0,
                    |ui| switch(ui, &mut edit.squad).changed(),
                );
                if flipped == Some(true) {
                    edit.has_squad = true;
                    changed = true;
                }
            });
            g.section("Dialogs", true, None, |g| {
                if g.row("Opening Dialog", "The dialog it opens with (start_dialog), captioned with its first line.", flags(own, "dialog", PropFlags::NONE), 0, |ui| {
                    local_list_picker(ui, "character.dialog", &mut edit.start_dialog, &lists.dialogs, &lists.dialogs, Some("None"), !lists.loaded)
                }) == Some(true)
                {
                    changed = true;
                }
                messages(g, own, "dialog");
            });
            g.section("Starting Kit", true, None, |g| {
                g.block(|ui| {
                    if draw_kit(ui, edit, inherited, &base, lists, own) {
                        changed = true;
                    }
                });
            });
            let trader = edit.trade.is_trader();
            g.section("Trade", true, trader.then_some("Trader"), |g| {
                let inherited_trade = inherited.map(|b| b.trade.clone()).unwrap_or_default();
                if trader {
                    if trade_rows(g, &mut edit.trade, lists, own) {
                        changed = true;
                    }
                    g.block(|ui| {
                        if draw_stock(ui, &mut edit.trade, &inherited_trade, lists, own) {
                            changed = true;
                        }
                    });
                } else {
                    g.block(|ui| {
                        if draw_trade_off(ui, &mut edit.trade, &inherited_trade, &base) {
                            changed = true;
                        }
                    });
                }
            });
        });
        if changed {
            self.changed();
        }
        if base_changed {
            self.actions.push(Action::BaseChanged);
        }
        if preview {
            self.actions.push(Action::Preview);
        }
    }
}

/// The rank by the linked game's names and the exact number beside it (`RankPicker`).
fn rank_row(ui: &mut Ui, edit: &mut Character, lists: &ListsView) -> bool {
    let mut changed = false;
    let number = em(6.0);
    let ranks = &lists.ranks;
    if !ranks.is_empty() {
        let combo_w = (ui.available_width() - number - 8.0).max(em(6.0));
        let cur = ranks.iter().rposition(|r| edit.rank >= r.from);
        let name = |e: &gz_nq::PickerEntry| Indexed::label(e);
        let preview = if !edit.has_rank {
            "Not set".to_owned()
        } else {
            cur.and_then(|i| ranks.get(i)).map_or_else(|| "Below the lowest rank".to_owned(), |r| name(&r.entry))
        };
        let rows: Vec<String> = ranks
            .iter()
            .enumerate()
            .map(|(i, r)| match ranks.get(i + 1) {
                Some(next) => format!("{}   {}-{}", name(&r.entry), r.from, next.from - 1),
                None => format!("{}   {}+", name(&r.entry), r.from),
            })
            .collect();
        let picked = cell(ui, combo_w, |ui| local_combo(ui, "character.rank", &preview, false, &rows, if edit.has_rank { cur } else { None }));
        if let Some(i) = picked
            && let Some(r) = ranks.get(i)
        {
            // the number stays when it already is that rank
            if !edit.has_rank || Some(i) != cur {
                edit.rank = r.from;
            }
            edit.has_rank = true;
            changed = true;
        }
    }
    let number_w = if ranks.is_empty() { ui.available_width() } else { number };
    let typed = cell(ui, number_w, |ui| integer(ui, &mut edit.rank, 10.0, "").changed());
    if typed {
        edit.has_rank = true;
        changed = true;
    }
    changed
}

/// Money: from, to and Never runs out on the row; empty while the character sets none, the
/// base's amounts as the hints (`MoneyRow`, `MoneyField`).
fn money_row(ui: &mut Ui, edit: &mut Character, inherited: Option<&Character>) -> bool {
    let ctx = ui.ctx().clone();
    let (base_min, base_max, base_inf) = inherited.filter(|b| b.has_money).map_or((0, 0, false), |b| (b.money_min, b.money_max, b.money_infinite));
    let number = em(5.5);
    let check_label = "Never runs out";
    let check_w = metrics::FRAME + 8.0 + text::width(&ctx, &Font::Body.id(), check_label);
    let to_w = text::width(&ctx, &Font::Body.id(), "to") + 8.0;
    let fits = number * 2.0 + to_w + 16.0 + check_w <= ui.available_width();
    let secondary = palette(&ctx).text_secondary;
    let mut changed = false;
    let adopt = |edit: &mut Character| {
        if !edit.has_money {
            edit.has_money = true;
            edit.money_min = base_min;
            edit.money_max = base_max;
            edit.money_infinite = base_inf;
        }
    };
    let numbers = |ui: &mut Ui, edit: &mut Character, changed: &mut bool| {
        let (min, max) = if edit.has_money { (Some(i64::from(edit.money_min)), Some(i64::from(edit.money_max))) } else { (None, None) };
        if let Some(v) = local_number_text(ui, "character.money_min", min, &base_min.to_string(), number) {
            adopt(edit);
            edit.money_min = i32::try_from(v.unwrap_or(0).max(0)).unwrap_or(i32::MAX);
            *changed = true;
        }
        let _ = text::label(ui, "to", Font::Body, secondary, Clip::End);
        if let Some(v) = local_number_text(ui, "character.money_max", max, &base_max.to_string(), number) {
            adopt(edit);
            edit.money_max = i32::try_from(v.unwrap_or(0).max(0)).unwrap_or(i32::MAX);
            *changed = true;
        }
    };
    let check = |ui: &mut Ui, edit: &mut Character, changed: &mut bool| {
        let mut infinite = if edit.has_money { edit.money_infinite } else { base_inf };
        let before = infinite;
        let _ = local_check(ui, "character.money_infinite", check_label, &mut infinite, check_w);
        if infinite != before {
            if !edit.has_money {
                edit.has_money = true;
                edit.money_min = base_min;
                edit.money_max = base_max;
            }
            edit.money_infinite = infinite;
            *changed = true;
        }
    };
    if fits {
        numbers(ui, edit, &mut changed);
        ui.add_space(8.0);
        check(ui, edit, &mut changed);
    } else {
        // the box goes under the numbers when the row is too narrow for it
        ui.vertical(|ui| {
            ui.horizontal(|ui| numbers(ui, edit, &mut changed));
            check(ui, edit, &mut changed);
        });
    }
    changed
}

/// The starting kit: what the base already gives (shown, not editable), then this character's
/// rows; Add Item (`DrawKit`). True when something changed.
fn draw_kit(ui: &mut Ui, edit: &mut Character, inherited: Option<&Character>, base: &str, lists: &ListsView, own: &[String]) -> bool {
    let p = palette(ui.ctx());
    let mut changed = false;
    let given: &[Supply] = inherited.map_or(&[], |b| b.supplies.as_slice());
    if given.is_empty() && edit.supplies.is_empty() {
        let _ = text::status(ui, Tone::Muted, "No starting kit: the NPC spawns with what its community gives.");
    } else {
        let total = ui.available_width();
        let (count_w, action_w) = (em(5.0), metrics::ICON_BUTTON);
        let item_w = (total - count_w - action_w - 16.0).max(em(6.0));
        let _ = table_headers(ui, &[Column::new("Item", item_w + 8.0), Column::new("Count", count_w + 8.0), Column::new("", action_w)]);
        for (i, row) in given.iter().enumerate() {
            ui.push_id(("inherited", i), |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(TABLE_ROW);
                    let label = format!("{}  - from {base}", item_label(&lists.items, &row.section));
                    cell(ui, item_w, |ui| {
                        let _ = text::label(ui, &label, Font::Body, p.text_muted, Clip::End)
                            .on_hover_text(format!("From {base}. A row of the same item below changes the count; a count of 0 takes it out."));
                    });
                    cell(ui, count_w, |ui| {
                        let _ = text::label(ui, &row.count.to_string(), Font::Body, p.text_muted, Clip::End);
                    });
                });
                row_rule(ui);
            });
        }
        let mut remove = None;
        let inherits = !given.is_empty();
        for (i, row) in edit.supplies.iter_mut().enumerate() {
            ui.push_id(("own", i), |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(TABLE_ROW);
                    if cell(ui, item_w, |ui| item_picker(ui, &format!("character.kit.{i}"), &mut row.section, lists, None)) {
                        changed = true;
                    }
                    if cell(ui, count_w, |ui| integer(ui, &mut row.count, 0.1, "").changed()) {
                        changed = true;
                    }
                    // 0 stays: with a base it means "not this one". Below zero never meant anything
                    row.count = row.count.max(0);
                    if row.count == 0 && !inherits {
                        row.count = 1;
                    }
                    if icon_button(ui, Icon::Delete, "Take the row out", false, true).clicked() {
                        remove = Some(i);
                    }
                });
                row_rule(ui);
            });
        }
        if let Some(i) = remove
            && i < edit.supplies.len()
        {
            edit.supplies.remove(i);
            changed = true;
        }
    }
    status_lines(ui, own, "kit");
    ui.add_space(2.0);
    if Button::new("Add Item").show(ui).clicked() {
        edit.supplies.push(Supply::new("", 1));
        changed = true;
    }
    changed
}

/// The shop's rows: what it sells and buys, and its prices (`DrawTradeRows`).
fn trade_rows(g: &mut Grid<'_>, t: &mut Trade, lists: &ListsView, own: &[String]) -> bool {
    let mut changed = false;
    let cats: &[String] = &lists.trade_categories;
    let busy = !lists.loaded;
    let help = "Besides its stock, which it always sells: the categories of the linked game's trade presets it sells when it has them. None: only the stock.";
    if g.row("Sells", help, flags(own, "trade", PropFlags::NONE), 0, |ui| local_multi_combo(ui, "character.sells", &mut t.sells, cats, "Only its stock", busy)) == Some(true) {
        changed = true;
    }
    if g.row("Buys", "The categories it buys from the player. None: it buys nothing.", PropFlags::NONE, 0, |ui| {
        local_multi_combo(ui, "character.buys", &mut t.buys, cats, "Nothing", busy)
    }) == Some(true)
    {
        changed = true;
    }
    if g.row("Markup", "What it asks, as a share of an item's cost: 1 is the cost, 1.5 half as much again.", PropFlags::NONE, 0, |ui| number(ui, &mut t.markup, 0.01, "").changed())
        == Some(true)
    {
        changed = true;
    }
    let help = "What it pays for an item, as a share of its cost - over the factors the game gives single items, the way the game's own traders do: 0.3 is about a third.";
    if g.row("Pays", help, PropFlags::NONE, 0, |ui| number(ui, &mut t.pays, 0.01, "").changed()) == Some(true) {
        changed = true;
    }
    if g.row("Restock", "Game hours until the stock comes back.", PropFlags::NONE, 0, |ui| integer(ui, &mut t.restock, 0.2, "h").changed()) == Some(true) {
        changed = true;
    }
    changed
}

/// The stock: what is on its shelf after every restock, each line with its chance; Add Item and
/// Not a Trader (`DrawStock`).
fn draw_stock(ui: &mut Ui, t: &mut Trade, inherited: &Trade, lists: &ListsView, own: &[String]) -> bool {
    let p = palette(ui.ctx());
    let mut changed = false;
    let _ = text::label(ui, "Stock", Font::Body, p.text_secondary, Clip::End)
        .on_hover_text("What is on the shelf after every restock: an item, how many, and the chance that it is there.");
    if t.stock.is_empty() {
        let _ = text::status(ui, Tone::Muted, "Nothing on the shelf: it sells only what its categories let it sell of what it carries.");
    } else {
        let total = ui.available_width();
        let (count_w, chance_w, action_w) = (em(4.5), em(4.5), metrics::ICON_BUTTON);
        let item_w = (total - count_w - chance_w - action_w - 24.0).max(em(6.0));
        let _ = table_headers(ui, &[Column::new("Item", item_w + 8.0), Column::new("Count", count_w + 8.0), Column::new("Chance", chance_w + 8.0), Column::new("", action_w)]);
        let mut remove = None;
        for (i, row) in t.stock.iter_mut().enumerate() {
            ui.push_id(("stock", i), |ui| {
                ui.horizontal(|ui| {
                    ui.set_height(TABLE_ROW);
                    if cell(ui, item_w, |ui| item_picker(ui, &format!("character.stock.{i}"), &mut row.section, lists, None)) {
                        changed = true;
                    }
                    if cell(ui, count_w, |ui| integer(ui, &mut row.count, 0.1, "").changed()) {
                        changed = true;
                    }
                    row.count = row.count.max(1);
                    if cell(ui, chance_w, |ui| number(ui, &mut row.chance, 0.01, "").changed()) {
                        changed = true;
                    }
                    if icon_button(ui, Icon::Delete, "Take the row out", false, true).clicked() {
                        remove = Some(i);
                    }
                });
                row_rule(ui);
            });
        }
        if let Some(i) = remove
            && i < t.stock.len()
        {
            t.stock.remove(i);
            changed = true;
        }
    }
    status_lines(ui, own, "trade");
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        if Button::new("Add Item").show(ui).clicked() {
            t.stock.push(TradeRow::default());
            changed = true;
        }
        if Button::new("Not a Trader").tip("The NPC keeps no shop: it trades like any stalker of its community.").show(ui).clicked() {
            let base_trades = inherited.is_trader();
            *t = Trade::default();
            // with a base that trades, saying nothing would bring its shop back
            t.declared = base_trades;
            t.off = base_trades;
            changed = true;
        }
    });
    changed
}

/// Not a trader, or trading as the base does: a line saying which, and the way to change it
/// (`DrawTradeOff`).
fn draw_trade_off(ui: &mut Ui, t: &mut Trade, inherited: &Trade, base: &str) -> bool {
    let mut changed = false;
    let from_base = !t.declared && inherited.is_trader();
    let line = if from_base {
        format!("Trades as {base} does: {}.", trade_summary(inherited))
    } else if t.declared && t.off && inherited.is_trader() {
        format!("Not a trader, although {base} is one.")
    } else {
        "Not a trader: it trades like any stalker of its community, with what it carries.".to_owned()
    };
    let _ = text::prose(ui, &line);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        let label = if from_base { "Change Trade" } else { "Make a Trader" };
        let tip = "A shop of its own: what it sells and buys, its stock and its prices. The game uses it whatever the NPC is doing.";
        if Button::new(label).label_set(&["Change Trade", "Make a Trader"]).tip(tip).show(ui).clicked() {
            *t = if inherited.is_trader() { inherited.clone() } else { Trade::default() };
            t.declared = true;
            t.off = false;
            changed = true;
        }
        if from_base && Button::new("Not a Trader").tip("Takes the base's shop away from this character.").show(ui).clicked() {
            *t = Trade { declared: true, off: true, ..Trade::default() };
            changed = true;
        }
        if t.declared && t.off && Button::new("Clear").tip(if base.is_empty() { "Says nothing about trade" } else { "Trades as the base does" }).show(ui).clicked() {
            *t = Trade::default();
            changed = true;
        }
    });
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shop_is_summed_up_in_one_line() {
        let t = Trade { declared: true, sells: vec!["weapon".into(), "food".into()], markup: 1.5, pays: 0.3, ..Trade::default() };
        assert_eq!(trade_summary(&t), "sells weapon, food, a stock of 0 item(s), asks 1.5 and pays 0.3 of the cost");
        assert_eq!(trade_summary(&Trade::default()), "sells its stock only, a stock of 0 item(s), asks 1 and pays 0.3 of the cost");
    }

    #[test]
    fn an_item_reads_by_its_name() {
        let items = Indexed::new(vec![gz_nq::PickerEntry { id: "bandage".into(), name: "Bandage".into(), extra: String::new() }], 1);
        assert_eq!(item_label(&items, "bandage"), "Bandage  (bandage)");
        assert_eq!(item_label(&items, "medkit"), "medkit");
    }
}
