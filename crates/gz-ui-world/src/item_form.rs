//! The drawing of the Items tab: the list, the form and the item as the game will show it.
//!
//! The form is the kit's property grid over the model of `item_model`: the catalog's rows of the
//! item's family, each with an editor for its type, the inherited value muted until the author
//! sets one and a Reset arrow that takes the item's own away. What a frame asks for is collected
//! as [`Edit`]s and applied after the grid is drawn.

use egui::{Align, Align2, Color32, CornerRadius, Id, Layout, Rect, Sense, Ui, UiBuilder, Vec2, pos2};
use gz_items::{Icon as ItemIcon, KeyType, OwnedSection, Scene, SetValue};
use gz_ui::grid::{Grid, chips, switch, text_block};
use gz_ui::list::list;
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{
    Col, Font, Icon, ItemRows, PropFlags, PropertyGrid, SearchField, TipExt, Tone, Tx, icon_button, icons,
    metrics, palette, tr,
};

use crate::item_model::{self, OtherRow, Own, Place, Row};
use crate::items::{Action, IconSource, ItemsDoc, Pick, first_name, game_keys, is_picture, list_name};
use crate::local::{local_add_field, local_dirty_dot, local_value_combo, local_value_text};
use crate::picker::list_picker;

const ROLE: &str = "item";
/// The grid of In the Hands' own rows.
const HUD_ROLE: &str = "item_hud";
/// How far In the Hands' rows stand in from the form's.
const HUD_INDENT: f32 = 14.0;
const LABEL_FRAC: f32 = 0.34;
/// A list row's icon: fitted into this many points.
const THUMB: Vec2 = Vec2::new(30.0, 22.0);
/// A cell of the inventory, pixels.
pub(crate) const CELL: f32 = 50.0;
/// The title of the form's Icon section.
const ICON_TITLE: &str = "Icon";

/// What the drawing asked for, applied once the grid is drawn.
pub(crate) enum Edit {
    Set {
        scope: String,
        key: String,
        value: SetValue,
    },
    Reset {
        scope: String,
        key: String,
    },
    Unset {
        scope: String,
        key: String,
    },
    Add {
        scope: String,
        key: String,
        member: String,
        now: Vec<String>,
    },
    Drop {
        scope: String,
        key: String,
        member: String,
        now: Vec<String>,
    },
    /// Any other change of the item (its identity, texts, icon, use scene ...).
    Item(Box<dyn FnOnce(&mut gz_items::Item)>),
    /// A text field, by `TEXTS`.
    Text(usize, String),
    Act(Action),
    HudRow(String),
}

/// The tone of a value by where it comes from: the item's own in the text colour.
fn from_text(row: &Row) -> Tx<'static> {
    match (row.from, &row.base) {
        (_, _) if matches!(row.own, Own::Set(_) | Own::Members { .. }) => Tx::from("Set by this item"),
        ("base", Some(b)) => tr!("From {base}", base = b.as_str()),
        ("base", None) => Tx::from("From the base"),
        ("game", _) => Tx::from("The game's"),
        ("module", _) => Tx::from("From the module's configs"),
        ("item", _) => Tx::from("From another item of the module"),
        _ => Tx::from("Not set"),
    }
}

/// A title or a unit of the game's item catalog, in the window's language.
fn cat(en: &str) -> Tx<'static> {
    Tx::ui_in("item", en.to_owned())
}

/// A row's label: the catalog's title, with the key after it where rows share one; the key
/// alone for a key the catalog gives no title.
pub(crate) fn row_label(row: &Row) -> Tx<'static> {
    if row.title == row.key {
        Tx::raw(row.key.clone())
    } else if row.keyed {
        tr!("{title} ({key})", title = cat(&row.title), key = row.key.as_str())
    } else {
        cat(&row.title)
    }
}

/// The keys of the item's texts: Name and Description owns them, so their rows stand under its
/// More and not in the catalog's group.
fn text_key(row: &Row) -> bool {
    row.scope == gz_items::catalog::ITEM
        && [game_keys::NAME, game_keys::SHORT, game_keys::DESCRIPTION].contains(&row.key.as_str())
}

/// Paints an item's icon fitted into `rect`, keeping its proportions; the kind's glyph while there
/// is none (or it is still read). Returns the rectangle it took. The Items and the Economy tab
/// draw an item the same way.
pub(crate) fn paint_item_icon(
    ui: &Ui,
    rect: Rect,
    src: Option<&IconSource>,
    editor: &std::sync::Arc<gz_editor::Editor>,
    pictures: Option<&std::sync::Arc<crate::item_lists::Pictures>>,
    sheets: Option<&std::sync::Arc<crate::item_lists::IconSheets>>,
) -> Rect {
    let p = palette(ui.ctx());
    let glyph = |ui: &Ui| {
        icons::draw(
            ui.painter(),
            Icon::Item,
            Rect::from_center_size(
                rect.center(),
                Vec2::splat(rect.height().min(metrics::GLYPH_ROW * 2.0) * 0.8),
            ),
            p.text_secondary,
        );
        rect
    };
    let fit = |w: f32, h: f32| -> Rect {
        let s = (rect.width() / w.max(1.0)).min(rect.height() / h.max(1.0));
        Rect::from_center_size(rect.center(), Vec2::new(w * s, h * s))
    };
    match src {
        Some(IconSource::Picture { path, stamp }) => {
            match pictures.and_then(|pics| pics.get(editor, path, *stamp)) {
                Some((tex, size)) => {
                    let r = fit(size[0] as f32, size[1] as f32);
                    ui.painter().image(
                        tex,
                        r,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                    r
                }
                None => glyph(ui),
            }
        }
        Some(IconSource::Cell { sheet, x, y, w, h }) => {
            let px = [*x as f32 * CELL, *y as f32 * CELL, *w as f32 * CELL, *h as f32 * CELL];
            // the project's own texture first, then the game's
            match sheets.and_then(|s| s.cell(editor, sheet, px)) {
                Some((tex, uv)) => {
                    let r = fit(*w as f32, *h as f32);
                    ui.painter().image(tex, r, uv, Color32::WHITE);
                    r
                }
                None => glyph(ui),
            }
        }
        None => glyph(ui),
    }
}

impl ItemsDoc {
    // ---- icons ----------------------------------------------------------------------------------

    /// Paints an icon fitted into `rect`: [`paint_item_icon`] with the tab's pictures and sheets.
    pub(crate) fn paint_icon(&self, ui: &Ui, rect: Rect, src: Option<&IconSource>) -> Rect {
        paint_item_icon(ui, rect, src, &self.editor, self.pictures.as_ref(), self.icon_sheets.as_ref())
    }

    /// Whether the edited item's icon stands drawn: its picture or its sheet is read (the first
    /// ask starts the read).
    pub(crate) fn icon_drawn(&self) -> bool {
        match self.edit_icon() {
            Some(IconSource::Picture { path, stamp }) => {
                self.pictures.as_ref().and_then(|p| p.get(&self.editor, &path, stamp)).is_some()
            }
            Some(IconSource::Cell { sheet, x, y, w, h }) => {
                let px = [x as f32 * CELL, y as f32 * CELL, w as f32 * CELL, h as f32 * CELL];
                self.icon_sheets.as_ref().and_then(|s| s.cell(&self.editor, &sheet, px)).is_some()
            }
            None => false,
        }
    }

    /// The icon of the edited item: its own, or what its section resolves to.
    pub(crate) fn edit_icon(&self) -> Option<IconSource> {
        let root = std::path::PathBuf::from(self.root_path());
        let main = self.res.as_ref().map(|r| &r.main);
        let get = |k: &str| main.and_then(|m| m.get(k)).and_then(|r| r.value.clone());
        crate::items::item_icon(&self.editor, &root, &self.edit, &get)
    }

    pub(crate) fn root_path(&self) -> String {
        self.editor.project_opt().map(|s| s.root.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// The icon and name of a game item a row names, worked out once per section.
    fn section_look(&mut self, section: &str) -> Option<IconSource> {
        let key = section.trim().to_ascii_lowercase();
        if let Some(v) = self.section_icons.get(&key) {
            return v.clone();
        }
        let found = self.roster().and_then(|r| crate::items::section_icon(&r.composition.ltx, &key));
        self.section_icons.insert(key, found.clone());
        found
    }

    // ---- the list -------------------------------------------------------------------------------

    pub(crate) fn draw_list(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        ui.horizontal(|ui| {
            let w = (ui.available_width() - metrics::ICON_BUTTON - 6.0).max(em(4.0));
            let _ = SearchField::new(&mut self.list_filter)
                .hint("Search items")
                .width(w)
                .id_salt("gz-ui-world.items.search")
                .show(ui);
            let resp = icon_button(ui, Icon::Plus, "Add an item", false, true);
            let menu = Id::unique("gz-ui-world.items.add");
            if resp.clicked() || gz_ui::menu::take_popup_request(&ctx, "items.add") {
                egui::Popup::open_id(&ctx, menu);
            }
            let mut pick = None;
            let _ = gz_ui::menu::menu_popup(&ctx, menu, resp.rect, ui.layer_id(), |ui| {
                if gz_ui::menu_item(
                    ui,
                    "New Item...",
                    gz_ui::MenuItem {
                        icon: Icon::Item,
                        tip: "An item of the module's own that starts from one of the game's".into(),
                        ..gz_ui::MenuItem::default()
                    },
                ) {
                    pick = Some(Pick::New);
                }
                if gz_ui::menu_item(
                    ui,
                    "Change a Game Item...",
                    gz_ui::MenuItem {
                        icon: Icon::Rename,
                        tip: "What the module changes of an item of the game".into(),
                        ..gz_ui::MenuItem::default()
                    },
                ) {
                    pick = Some(Pick::Change);
                }
            });
            if let Some(pick) = pick {
                egui::Popup::close_id(&ctx, menu);
                self.actions.push(Action::Choose(pick));
            }
        });
        ui.add_space(4.0);
        let Some(loaded) = self.loaded.as_ref() else {
            gz_ui::state::busy_line(ui, "Reading the items...");
            return;
        };
        let roster = loaded.roster.clone();
        let filter = self.list_filter.clone();
        let rows: Vec<(usize, bool, String, String)> = roster
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                let section =
                    roster.composition.applied.get(i).map(|a| a.section.clone()).unwrap_or_default();
                let name = list_name(&e.item, &self.item_lists, &section);
                gz_ui::search::matches(&filter, &format!("{} {} {}", e.item.file, section, name))
                    .then(|| (i, item_model::is_change(&e.item), name, section))
            })
            .collect();
        let mut picked: Option<String> = None;
        let mut new_clicked = false;
        let sel = self.sel.clone();
        let dirty = self.dirty;
        let this: &ItemsDoc = self;
        list(ui, "gz-ui-world.items.list", Vec2::ZERO, |l| {
            for (heading, change) in [("New", false), ("Changes", true)] {
                let of: Vec<&(usize, bool, String, String)> = rows.iter().filter(|r| r.1 == change).collect();
                if of.is_empty() {
                    continue;
                }
                let _ = text::label(l.ui(), heading, Font::Small, p.text_muted, Clip::End);
                for (i, _, name, section) in of {
                    let Some(e) = roster.entries.get(*i) else { continue };
                    let selected = sel.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&e.item.file));
                    let icon = loaded.icons.get(*i).cloned().flatten();
                    let worst = loaded.problems.get(*i).and_then(|ps| {
                        if ps.iter().any(gz_items::Issue::is_error) {
                            Some(Tone::Error)
                        } else {
                            ps.iter()
                                .any(|x| x.severity == gz_base::Severity::Warning)
                                .then_some(Tone::Warning)
                        }
                    });
                    let resp = l.row(&e.item.file, selected, |ui| {
                        let r = ui.max_rect();
                        let cy = r.center().y;
                        let thumb = Rect::from_min_size(pos2(r.min.x, cy - THUMB.y * 0.5), THUMB);
                        let _ = this.paint_icon(ui, thumb, icon.as_ref());
                        let template = e.item.is_abstract();
                        let right = if template { em(4.5) } else { 0.0 }
                            + if selected && dirty { 14.0 } else { 0.0 }
                            + if worst.is_some() { 12.0 } else { 0.0 }
                            + 6.0;
                        let x = r.min.x + THUMB.x + 6.0;
                        let room = (r.max.x - x - right).max(0.0);
                        let font = Font::Body.id();
                        let shown = if name.is_empty() { section.as_str() } else { name.as_str() };
                        let name_w = text::width(ui.ctx(), &font, Tx::raw(shown)).min(room);
                        text::paint(
                            ui.painter(),
                            &font,
                            pos2(x, cy),
                            Align2::LEFT_CENTER,
                            p.text,
                            Tx::raw(shown),
                            Some(room),
                            Clip::End,
                        );
                        if !name.is_empty() && room - name_w > em(3.0) {
                            text::paint(
                                ui.painter(),
                                &font,
                                pos2(x + name_w + 8.0, cy),
                                Align2::LEFT_CENTER,
                                p.text_muted,
                                Tx::raw(section.as_str()),
                                Some((room - name_w - 8.0).max(0.0)),
                                Clip::End,
                            );
                        }
                        let mut end = r.max.x - 6.0;
                        if let Some(tone) = worst {
                            ui.painter().circle_filled(pos2(end - 3.0, cy), 3.5, p.tone(tone));
                            end -= 12.0;
                        }
                        if selected && dirty {
                            local_dirty_dot(ui.painter(), pos2(end - 4.0, cy), ui.ctx());
                            end -= 14.0;
                        }
                        if template {
                            let badge = Rect::from_min_max(pos2(end - em(4.5), r.min.y), pos2(end, r.max.y));
                            let mut child = ui.new_child(
                                UiBuilder::new()
                                    .id_salt(("gz-ui-world.item_template", *i))
                                    .max_rect(badge)
                                    .layout(Layout::left_to_right(Align::Center)),
                            );
                            gz_ui::state::badge(&mut child, "Template", Tone::Muted);
                        }
                    });
                    if resp.tip(Tx::raw(e.item.file.clone())).clicked() && !selected {
                        picked = Some(e.item.file.clone());
                    }
                }
            }
            if roster.entries.is_empty() {
                if gz_ui::state::empty_state(
                    l.ui(),
                    Icon::Item,
                    "No items",
                    "The module's own items and its changes of the game's live here.",
                    Some("New Item...".into()),
                ) {
                    new_clicked = true;
                }
            } else if rows.is_empty() {
                let _ = text::status(l.ui(), Tone::Muted, "No item matches.");
            }
        });
        if new_clicked {
            self.actions.push(Action::Choose(Pick::New));
        }
        if let Some(file) = picked {
            self.actions.push(Action::Select(file));
        }
    }

    // ---- the form -------------------------------------------------------------------------------

    fn draw_header(&mut self, ui: &mut Ui, width: f32) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let icon_box = Vec2::new(64.0 * 1.4, 64.0);
        let section = self.res.as_ref().map(|r| r.section.clone()).unwrap_or_default();
        let name = list_name(&self.edit, &self.item_lists, &section);
        let title = if name.is_empty() {
            Tx::raw(if section.is_empty() { self.edit.file.clone() } else { section.clone() })
        } else {
            Tx::raw(name)
        };
        let icon = self.edit_icon();
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(icon_box, Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(4), p.inset);
            let _ = self.paint_icon(ui, rect.shrink(4.0), icon.as_ref());
            ui.add_space(12.0);
            ui.vertical(|ui| {
                ui.set_width((width - icon_box.x - 20.0).max(em(4.0)));
                let _ = text::label(ui, &title, Font::Title, p.text, Clip::End);
                let kind = if item_model::is_change(&self.edit) {
                    tr!("A change of {section}", section = section.as_str())
                } else if section.is_empty() {
                    Tx::from("A new item")
                } else {
                    tr!("A new item: {section}", section = section.as_str())
                };
                let kind = if self.form.family_title.is_empty() {
                    kind
                } else {
                    tr!("{kind} · {family}", kind = kind, family = cat(&self.form.family_title))
                };
                let _ = text::label(ui, kind, Font::Body, p.text_secondary, Clip::End);
                let _ = text::label(
                    ui,
                    Tx::raw(self.edit.file.clone()),
                    Font::Body,
                    p.text_secondary,
                    Clip::Start,
                );
                if self.edit.is_abstract() {
                    gz_ui::state::badge(ui, "Template", Tone::Muted);
                }
            });
        });
        for (_, tone, line) in self.places.iter().filter(|(pl, _, _)| *pl == Place::Head) {
            let _ = text::status(ui, *tone, Tx::ui(line.as_str()));
        }
        if !self.status.is_empty() {
            let _ = text::status(ui, Tone::Error, Tx::ui(self.status.as_str()));
        }
        if !self.conflicts.is_empty() {
            let n = self.conflicts.len();
            let head = tr!(
                "Changed on disk too: {n} {n:value|values}. The form keeps its own; Take Theirs takes the file's.",
                n = n
            );
            let lines: Vec<String> = self.conflicts.iter().map(crate::char_form::conflict_line).collect();
            let items: Vec<(Tx<'_>, Option<Tx<'_>>)> =
                lines.iter().map(|l| (Tx::ui(l.as_str()), Some(Tx::from("Take Theirs")))).collect();
            if let Some(i) = gz_ui::state::banner_list(ui, Tone::Warning, head, &items) {
                self.take_theirs(i);
            }
        }
        ui.add_space(6.0);
    }

    pub(crate) fn draw_form(&mut self, ui: &mut Ui, width: f32) {
        self.draw_header(ui, width);
        let mut edits: Vec<Edit> = Vec::new();
        let change = item_model::is_change(&self.edit);
        // what the rows draw from, taken apart from the doc the closures borrow
        let form = std::mem::take(&mut self.form);
        let mut looks: Vec<(String, Option<IconSource>)> = Vec::new();
        for row in
            form.groups.iter().chain(form.hud.iter().flat_map(|(_, g)| g.iter())).flat_map(|g| g.rows.iter())
        {
            if row.ty == KeyType::Section
                && let Some(v) = row.shown()
            {
                for s in item_model::members_of(&v) {
                    let look = self.section_look(&s);
                    looks.push((s, look));
                }
            }
        }
        let this: &ItemsDoc = self;
        PropertyGrid::new(ROLE).label_frac(LABEL_FRAC).show(ui, |g| {
            this.identity(g, change, &mut edits);
            let ids: Vec<&Row> =
                form.groups.iter().flat_map(|g| g.rows.iter()).filter(|r| text_key(r)).collect();
            this.texts_section(g, &ids, change, &looks, &mut edits);
            // the catalog's group of the icon's keys is the Icon section's own fold: two sections
            // of one title would be one
            let (icon_keys, groups): (Vec<&item_model::Group>, Vec<&item_model::Group>) =
                form.groups.iter().partition(|g| g.title.eq_ignore_ascii_case(ICON_TITLE));
            this.icon_section(g, &icon_keys, change, &looks, &mut edits);
            for group in groups {
                if group.rows.iter().all(text_key) {
                    continue;
                }
                let open = group.rows.iter().any(|r| !text_key(r) && (r.common || r.own.is_own()));
                g.section(cat(&group.title), open, None, |g| {
                    this.group_rows(g, group, change, &looks, &mut edits)
                });
            }
            if let Some((hud, groups)) = &form.hud {
                g.section("In the Hands", true, None, |g| {
                    g.block(|ui| this.hud_grid(ui, hud, groups, &form.other, change, &looks, &mut edits));
                });
            }
            this.found_section(g, &mut edits);
            this.scene_section(g, &form, &mut edits);
            if !this.edit.sections.is_empty() {
                g.section("More Sections", true, Some(Tx::raw(this.edit.sections.len().to_string())), |g| {
                    this.sections_section(g, &mut edits)
                });
            }
            let own_other = form.other.iter().any(|o| o.scope == gz_items::catalog::ITEM);
            g.section("Other Keys", own_other, None, |g| {
                this.other_section(g, &form.other, gz_items::catalog::ITEM, &mut edits)
            });
        });
        self.form = form;
        self.apply(edits);
    }

    /// In the Hands' own rows, nested under its header: a grid of its own a step in, a rule down
    /// its left edge, its value column where the form's is - the HUD Section, the catalog's HUD
    /// groups, and the HUD keys the catalog lacks.
    #[allow(clippy::too_many_arguments)]
    fn hud_grid(
        &self,
        ui: &mut Ui,
        hud: &str,
        groups: &[item_model::Group],
        other: &[OtherRow],
        change: bool,
        looks: &[(String, Option<IconSource>)],
        edits: &mut Vec<Edit>,
    ) {
        let p = palette(ui.ctx());
        let outer = ui.available_rect_before_wrap();
        let width = outer.width().max(1.0);
        let frac = gz_ui::layout::get(ui.ctx(), &format!("grid.{ROLE}"), LABEL_FRAC).clamp(0.1, 0.9);
        let label_w = gz_ui::grid::label_width(width, frac);
        let inner_w = (width - HUD_INDENT).max(1.0);
        let inner_frac = ((label_w - HUD_INDENT) / inner_w).clamp(0.1, 0.9);
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-world.items.hud_grid")
                .max_rect(Rect::from_min_max(pos2(outer.min.x + HUD_INDENT, outer.min.y), outer.max))
                .layout(Layout::top_down(Align::Min)),
        );
        PropertyGrid::new(HUD_ROLE).label_frac(inner_frac).show(&mut child, |g| {
            let _ = g.row("HUD Section", "The section of the first-person model, the hands and the motions: the item's hud key names it.", PropFlags::READ_ONLY, 0, |ui| {
                gz_ui::grid::read_only(ui, Tx::raw(hud), Clip::End);
            });
            for group in groups {
                let open = group.rows.iter().any(|r| r.own.is_own());
                g.section(cat(&group.title), open, None, |g| self.group_rows(g, group, change, looks, edits));
            }
            let hud_other = other.iter().any(|o| o.scope == gz_items::catalog::HUD);
            g.section("Other Keys", hud_other, None, |g| self.other_section(g, other, gz_items::catalog::HUD, edits));
        });
        let used = child.min_rect();
        ui.painter().vline(
            outer.min.x + HUD_INDENT * 0.5 - 1.0,
            outer.min.y..=used.max.y,
            egui::Stroke::new(2.0, p.border_subtle),
        );
        ui.advance_cursor_after_rect(Rect::from_min_max(outer.min, pos2(outer.max.x, used.max.y)));
    }

    fn apply(&mut self, edits: Vec<Edit>) {
        let mut changed = false;
        for e in edits {
            match e {
                Edit::Set { scope, key, value } => {
                    item_model::set_own(self.changes_mut(&scope), &key, value);
                    changed = true;
                }
                Edit::Reset { scope, key } => {
                    item_model::reset(self.changes_mut(&scope), &key);
                    changed = true;
                }
                Edit::Unset { scope, key } => {
                    item_model::take_away(self.changes_mut(&scope), &key);
                    changed = true;
                }
                Edit::Add { scope, key, member, now } => {
                    let change = item_model::is_change(&self.edit);
                    item_model::add_member(self.changes_mut(&scope), &key, &member, change, &now);
                    changed = true;
                }
                Edit::Drop { scope, key, member, now } => {
                    let change = item_model::is_change(&self.edit);
                    item_model::drop_member(self.changes_mut(&scope), &key, &member, change, &now);
                    changed = true;
                }
                Edit::Item(f) => {
                    f(&mut self.edit);
                    changed = true;
                }
                Edit::Text(i, t) => {
                    if let Some(slot) = self.texts.get_mut(i) {
                        *slot = t;
                    }
                    changed = true;
                }
                Edit::Act(a) => self.actions.push(a),
                Edit::HudRow(k) => self.hud_row = Some(k),
            }
        }
        if changed {
            // an empty HUD section says nothing
            if self.edit.hud.as_ref().is_some_and(|h| h.base.is_empty() && h.changes.is_empty())
                && self.base.as_ref().is_none_or(|b| b.hud.is_none())
            {
                self.edit.hud = None;
            }
            self.changed();
        }
    }

    fn messages(&self, g: &mut Grid<'_>, at: &Place) {
        for (pl, tone, line) in &self.places {
            if pl == at {
                g.message(*tone, Tx::ui(line.as_str()));
            }
        }
    }

    fn has_error(&self, at: &Place) -> bool {
        self.places.iter().any(|(pl, tone, _)| pl == at && *tone == Tone::Error)
    }

    fn field_place(name: &str) -> Place {
        Place::Field(name.to_owned())
    }

    // ---- Identity -------------------------------------------------------------------------------

    fn identity(&self, g: &mut Grid<'_>, change: bool, edits: &mut Vec<Edit>) {
        let module = if self.module.is_empty() { "<module>".to_owned() } else { self.module.clone() };
        g.section("Identity", true, None, |g| {
            if change {
                let target = self.edit.edit.clone().unwrap_or_default();
                let name = self.item_lists.game_item(&target).map(|x| x.name.clone()).filter(|n| !n.is_empty());
                g.block(|ui| {
                    let line = match name {
                        Some(n) => tr!("Changes {name} ({section})", name = Tx::raw(n), section = target.as_str()),
                        None => tr!("Changes {section}", section = target.as_str()),
                    };
                    let _ = text::prose(ui, line);
                });
                self.messages(g, &Self::field_place("edit"));
                return;
            }
            let hint = format!("{module}_{}", self.edit.id_or_stem());
            let own = self.edit.section.clone();
            let flags = if self.has_error(&Self::field_place("section")) { PropFlags::INVALID } else { PropFlags::NONE };
            if let Some((Some(typed), _)) = g.row("Section", tr!("The item's name in the game. Left empty it is {hint}.", hint = hint.as_str()), flags, 0, |ui| {
                local_value_text(ui, "item.section", own.as_deref(), Tx::raw(hint.clone()), ui.available_width())
            }) {
                let typed = typed.trim().to_owned();
                edits.push(Edit::Item(Box::new(move |it| it.section = (!typed.is_empty()).then_some(typed))));
            }
            self.messages(g, &Self::field_place("section"));
            let bases = self.edit.base.clone();
            let flags = if bases.is_empty() || self.has_error(&Self::field_place("base")) { PropFlags::REQUIRED } else { PropFlags::NONE };
            let labels: Vec<Tx<'static>> = bases
                .iter()
                .map(|b| match self.item_lists.game_item(b).map(|x| x.name.clone()).filter(|n| !n.is_empty()) {
                    Some(n) => Tx::raw(format!("{n}  ({b})")),
                    None => Tx::raw(b.clone()),
                })
                .collect();
            if let Some((removed, add)) = g.row("Starts From", "The item of the game it starts from: what it does not set comes from there. Several: a later one wins.", flags, 0, |ui| {
                ui.vertical(|ui| {
                    let removed = chips(ui, "item.base", &labels, true);
                    let add = gz_ui::Button::new(if bases.is_empty() { "Pick an Item..." } else { "Add..." })
                        .label_set(&["Pick an Item...", "Add..."])
                        .show(ui)
                        .clicked();
                    (removed, add)
                })
                .inner
            }) {
                if let Some(i) = removed {
                    edits.push(Edit::Item(Box::new(move |it| {
                        if i < it.base.len() {
                            it.base.remove(i);
                        }
                    })));
                }
                if add {
                    edits.push(Edit::Act(Action::Choose(Pick::Base)));
                }
            }
            self.messages(g, &Self::field_place("base"));
            g.section("More", self.edit.id.is_some() || self.edit.is_abstract.is_some(), None, |g| {
                let own = self.edit.id.clone();
                let stem = gz_characters::model::id_from_name(gz_items::model::stem(&self.edit.file));
                if let Some((Some(typed), _)) = g.row("Id", "a-z, 0-9 and _. Left empty it is the file's name.", PropFlags::NONE, 0, |ui| {
                    local_value_text(ui, "item.id", own.as_deref(), Tx::raw(stem.clone()), ui.available_width())
                }) {
                    let typed = typed.trim().to_owned();
                    edits.push(Edit::Item(Box::new(move |it| it.id = (!typed.is_empty()).then_some(typed))));
                }
                self.messages(g, &Self::field_place("id"));
                let mut on = self.edit.is_abstract();
                if g.row("Template", "A template only: its section and nothing else - no name, no icon, never in the game's lists.", PropFlags::INLINE, 0, |ui| switch(ui, &mut on).changed()) == Some(true) {
                    edits.push(Edit::Item(Box::new(move |it| it.is_abstract = on.then_some(true))));
                }
            });
        });
    }

    // ---- Name and Description -------------------------------------------------------------------

    fn texts_section(
        &self,
        g: &mut Grid<'_>,
        ids: &[&Row],
        change: bool,
        looks: &[(String, Option<IconSource>)],
        edits: &mut Vec<Edit>,
    ) {
        g.section("Name and Description", true, None, |g| {
            let row = |g: &mut Grid<'_>,
                       edits: &mut Vec<Edit>,
                       i: usize,
                       label: &'static str,
                       help: &'static str| {
                let hint = self.hints[i].clone();
                let own = self.texts[i].clone();
                let salt = format!("item.text.{i}");
                if let Some((Some(typed), _)) = g.row(label, help, PropFlags::NONE, 0, |ui| {
                    local_value_text(
                        ui,
                        &salt,
                        Some(own.as_str()),
                        Tx::raw(hint.clone()),
                        ui.available_width(),
                    )
                }) {
                    edits.push(Edit::Text(i, typed));
                }
            };
            row(g, edits, 0, "Name (rus)", "What the player reads in the inventory and the trade window.");
            row(g, edits, 1, "Name (eng)", "Left empty, an English install reads the Russian name.");
            self.messages(g, &Self::field_place("name"));
            g.block(|ui| {
                for (i, label, help) in [
                    (4, "Description (rus)", "What the inventory's tooltip says of the item."),
                    (5, "Description (eng)", "Left empty, an English install reads the Russian description."),
                ] {
                    let mut text_ = self.texts[i].clone();
                    if text_block(ui, label, help, &mut text_, 2).changed() {
                        edits.push(Edit::Text(i, text_.clone()));
                    }
                    if text_.is_empty() && !self.hints[i].is_empty() {
                        let _ = text::label(
                            ui,
                            Tx::raw(self.hints[i].clone()),
                            Font::Small,
                            palette(ui.ctx()).text_muted,
                            Clip::End,
                        )
                        .tip("The game's text, shown while the item sets none");
                    }
                }
            });
            self.messages(g, &Self::field_place("description"));
            let open =
                !self.texts[2].is_empty() || !self.texts[3].is_empty() || ids.iter().any(|r| r.own.is_own());
            g.section("More", open, None, |g| {
                row(
                    g,
                    edits,
                    2,
                    "Short Name (rus)",
                    "The name on the inventory cell. Left empty it is the name.",
                );
                row(
                    g,
                    edits,
                    3,
                    "Short Name (eng)",
                    "Left empty, an English install reads the Russian short name.",
                );
                self.messages(g, &Self::field_place("short_name"));
                // the string ids themselves, for a text that exists already
                for row in ids {
                    self.key_row(g, row, change, looks, edits);
                }
            });
        });
    }

    // ---- Icon -----------------------------------------------------------------------------------

    #[allow(clippy::too_many_lines)]
    fn icon_section(
        &self,
        g: &mut Grid<'_>,
        keys: &[&item_model::Group],
        change: bool,
        looks: &[(String, Option<IconSource>)],
        edits: &mut Vec<Edit>,
    ) {
        let src = self.edit_icon();
        let open_keys = keys.iter().flat_map(|k| k.rows.iter()).any(|r| r.own.is_own());
        let draw_keys = |g: &mut Grid<'_>, edits: &mut Vec<Edit>| {
            for row in keys.iter().flat_map(|k| k.rows.iter()) {
                self.key_row(g, row, change, looks, edits);
            }
        };
        g.section(ICON_TITLE, true, None, |g| {
            g.block(|ui| {
                let p = palette(ui.ctx());
                let (cw, ch) = match &src {
                    Some(IconSource::Cell { w, h, .. }) => (*w as f32, *h as f32),
                    Some(IconSource::Picture { path, stamp }) => {
                        let px = self.pictures.as_ref().and_then(|pics| pics.get(&self.editor, path, *stamp)).map(|(_, s)| s);
                        let (w, h) = match &self.edit.icon {
                            Some(ItemIcon::Picture { w, h, .. }) => (*w, *h),
                            _ => (None, None),
                        };
                        let cells = px.map_or((w.unwrap_or(1), h.unwrap_or(1)), |s| gz_items::atlas::cells_of((s[0], s[1]), w, h));
                        (cells.0 as f32, cells.1 as f32)
                    }
                    None => (1.0, 1.0),
                };
                // cells of 50 px at 1 pt a pixel, fitted to the row
                let want = Vec2::new(cw * CELL, ch * CELL);
                let s = (ui.available_width() / want.x).min(1.0);
                let size = Vec2::new((want.x * s).max(CELL), (want.y * s).max(CELL * 0.6));
                let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
                ui.painter().rect_filled(rect, CornerRadius::same(4), p.inset);
                let _ = self.paint_icon(ui, rect.shrink(2.0), src.as_ref());
                let _ = resp.tip("The icon as the inventory shows it. Drop a picture on it to use that one.");
                let accepts = |pl: &gz_ui::drop::Payload| pl.files().iter().any(|f| is_picture(f));
                if let Some(payload) = gz_ui::drop::Zone::new("Item icon", rect, &accepts).take(ui) {
                    edits.push(Edit::Act(Action::Dropped(payload.files())));
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if gz_ui::Button::new("Choose Picture...").tip("A picture beside the asset: the module build bakes it into the module's icons").show(ui).clicked() {
                        edits.push(Edit::Act(Action::ChoosePicture));
                    }
                    if self.edit.icon.is_some()
                        && gz_ui::Button::new("Reset").icon(Icon::Undo).tip(if item_model::is_change(&self.edit) { "Back to the game's icon" } else { "Back to the base's icon" }).show(ui).clicked()
                    {
                        edits.push(Edit::Item(Box::new(|it| it.icon = None)));
                    }
                });
            });
            self.messages(g, &Self::field_place("icon"));
            match self.edit.icon.clone() {
                Some(ItemIcon::Picture { file, w, h, .. }) => {
                    let _ = g.row("Picture", "The picture file, beside the asset.", PropFlags::READ_ONLY, 0, |ui| gz_ui::grid::read_only(ui, Tx::raw(file.clone()), Clip::Start));
                    let px = match &src {
                        Some(IconSource::Picture { path, stamp }) => self.pictures.as_ref().and_then(|pics| pics.get(&self.editor, path, *stamp)).map(|(_, s)| s),
                        _ => None,
                    };
                    let auto = px.map(|s| gz_items::atlas::cells_of((s[0], s[1]), None, None));
                    for (label, own, hint, wide) in [("Width", w, auto.map(|a| a.0), true), ("Height", h, auto.map(|a| a.1), false)] {
                        let salt = if wide { "item.icon.w" } else { "item.icon.h" };
                        let own_text = own.map(|v| v.to_string());
                        if let Some((Some(typed), _)) = g.row(label, "In cells of 50 px. Left empty it is worked out from the picture.", PropFlags::NONE, 0, |ui| {
                            local_value_text(ui, salt, own_text.as_deref(), Tx::raw(hint.map(|h| h.to_string()).unwrap_or_default()), em(6.0))
                        }) {
                            let v = typed.trim().parse::<u32>().ok().filter(|v| (1..=64).contains(v));
                            edits.push(Edit::Item(Box::new(move |it| {
                                if let Some(ItemIcon::Picture { w, h, plain, .. }) = &mut it.icon {
                                    if wide { *w = v } else { *h = v }
                                    *plain = w.is_none() && h.is_none();
                                }
                            })));
                        }
                    }
                    if keys.iter().any(|k| !k.rows.is_empty()) {
                        g.section("More", open_keys, None, |g| draw_keys(g, edits));
                    }
                }
                Some(ItemIcon::Cell { texture, x, y, w, h }) => {
                    g.section("More", open_keys, None, |g| {
                        let tex = texture.clone();
                        if let Some((Some(typed), _)) = g.row("Texture", "The texture of the rectangle, by its game name. Left empty it is the game's icons.", PropFlags::NONE, 0, |ui| {
                            local_value_text(ui, "item.icon.texture", tex.as_deref(), Tx::raw(game_keys::DEFAULT_ICONS), ui.available_width())
                        }) {
                            let t = typed.trim().to_owned();
                            edits.push(Edit::Item(Box::new(move |it| {
                                if let Some(ItemIcon::Cell { texture, .. }) = &mut it.icon {
                                    *texture = (!t.is_empty()).then_some(t);
                                }
                            })));
                        }
                        for (n, label, v) in [(0, "X", Some(x)), (1, "Y", Some(y)), (2, "Width", w), (3, "Height", h)] {
                            let own = v.map(|v| v.to_string());
                            if let Some((Some(typed), _)) = g.row(label, "In cells of 50 px.", PropFlags::NONE, 0, |ui| {
                                local_value_text(ui, &format!("item.icon.cell.{n}"), own.as_deref(), Tx::EMPTY, em(6.0))
                            }) {
                                let v = typed.trim().parse::<u32>().ok();
                                edits.push(Edit::Item(Box::new(move |it| {
                                    if let Some(ItemIcon::Cell { x, y, w, h, .. }) = &mut it.icon {
                                        match n {
                                            0 => *x = v.unwrap_or(0),
                                            1 => *y = v.unwrap_or(0),
                                            2 => *w = v,
                                            _ => *h = v,
                                        }
                                    }
                                })));
                            }
                        }
                        draw_keys(g, edits);
                    });
                }
                _ => {
                    if keys.iter().any(|k| !k.rows.is_empty()) {
                        g.section("More", open_keys, None, |g| draw_keys(g, edits));
                    }
                }
            }
        });
    }

    // ---- the catalog's keys ---------------------------------------------------------------------

    fn group_rows(
        &self,
        g: &mut Grid<'_>,
        group: &item_model::Group,
        change: bool,
        looks: &[(String, Option<IconSource>)],
        edits: &mut Vec<Edit>,
    ) {
        let rows: Vec<&Row> = group.rows.iter().filter(|r| !text_key(r)).collect();
        for row in rows.iter().filter(|r| r.common) {
            self.key_row(g, row, change, looks, edits);
        }
        let more: Vec<&Row> = rows.iter().copied().filter(|r| !r.common).collect();
        if more.is_empty() {
            return;
        }
        // a group with nothing on its surface is its own fold
        if more.len() == rows.len() {
            for row in more {
                self.key_row(g, row, change, looks, edits);
            }
            return;
        }
        let open = more.iter().any(|r| r.own.is_own());
        g.section("More", open, Some(Tx::raw(more.len().to_string())), |g| {
            for row in more {
                self.key_row(g, row, change, looks, edits);
            }
        });
    }

    #[allow(clippy::too_many_lines)]
    fn key_row(
        &self,
        g: &mut Grid<'_>,
        row: &Row,
        change: bool,
        looks: &[(String, Option<IconSource>)],
        edits: &mut Vec<Edit>,
    ) {
        let place = Place::Key { scope: row.scope.clone(), key: row.key.clone() };
        let own = row.own.is_own();
        let mut flags = if own { PropFlags::MODIFIED } else { PropFlags::NONE };
        if row.required && row.resolved.is_none() && !matches!(row.own, Own::Set(_)) {
            flags = flags | PropFlags::REQUIRED;
        }
        if self.has_error(&place) {
            flags = flags | PropFlags::INVALID;
        }
        let help = if row.help.is_empty() {
            tr!("{key}\n{from}", key = row.key.as_str(), from = from_text(row))
        } else {
            tr!(
                "{key}\n{help}\n{from}",
                key = row.key.as_str(),
                help = Tx::ui_in("item_help", row.help.clone()),
                from = from_text(row)
            )
        };
        let salt = format!("item.key.{}.{}", row.scope, row.key);
        let (scope, key) = (row.scope.clone(), row.key.clone());
        let hud = row.scope == gz_items::catalog::HUD;
        let shown = row.shown().unwrap_or_default();
        let muted = Tx::raw(row.resolved.clone().unwrap_or_default());
        let unit_w = |ui: &Ui| {
            if row.unit.is_empty() {
                0.0
            } else {
                text::width(ui.ctx(), &Font::Body.id(), cat(&row.unit)) + 8.0
            }
        };
        let mut focus = false;
        let drawn = g.row(row_label(row), help, flags, 0, |ui| {
            let p = palette(ui.ctx());
            if row.own == Own::Unset {
                let was = row.before.clone().or_else(|| row.resolved.clone()).unwrap_or_default();
                let t = if was.is_empty() {
                    Tx::from("Taken away")
                } else {
                    tr!("Taken away (was {value})", value = was.as_str())
                };
                let _ = text::label(ui, t, Font::Body, p.text_muted, Clip::End)
                    .tip("The item takes the key away. The Reset arrow brings it back.");
                return None;
            }
            if row.list {
                let start = if change {
                    row.before.as_deref().or(row.resolved.as_deref())
                } else {
                    row.resolved.as_deref()
                };
                let now = item_model::members_now(
                    &row.own,
                    if matches!(row.own, Own::Members { .. }) { row.before.as_deref() } else { start },
                );
                let labels: Vec<Tx<'static>> = now.iter().map(|m| self.member_label(m)).collect();
                return ui
                    .vertical(|ui| {
                        let removed = chips(ui, &salt, &labels, true);
                        let added = local_add_field(ui, &salt, Tx::from("Add a member"), em(12.0));
                        if let Some(i) = removed {
                            return Some(Edit::Drop {
                                scope: scope.clone(),
                                key: key.clone(),
                                member: now[i].clone(),
                                now: now.clone(),
                            });
                        }
                        added.map(|m| Edit::Add {
                            scope: scope.clone(),
                            key: key.clone(),
                            member: m,
                            now: now.clone(),
                        })
                    })
                    .inner;
            }
            match &row.ty {
                KeyType::Bool => {
                    let mut on = item_model::is_on(&shown);
                    let r = switch(ui, &mut on);
                    if !own {
                        ui.add_space(8.0);
                        let _ = text::label(ui, from_text(row), Font::Small, p.text_muted, Clip::End);
                    }
                    focus |= r.clicked();
                    r.changed().then(|| Edit::Set {
                        scope: scope.clone(),
                        key: key.clone(),
                        value: SetValue::Bool(on),
                    })
                }
                KeyType::Token if !row.values.is_empty() => {
                    local_value_combo(ui, &salt, &shown, own, &row.values).map(|w| Edit::Set {
                        scope: scope.clone(),
                        key: key.clone(),
                        value: SetValue::Text(w),
                    })
                }
                ty @ (KeyType::Visual | KeyType::Texture | KeyType::Sound | KeyType::Particles) => {
                    let _ = ty;
                    let text_ = if shown.is_empty() { Tx::from("None") } else { Tx::raw(shown.clone()) };
                    let combo = gz_ui::Combo::new(("gz-ui-world.choose", salt.as_str()), text_)
                        .placeholder(!own || shown.is_empty());
                    let resp = combo.field(ui, false);
                    focus |= resp.clicked();
                    (resp.clicked() || gz_ui::menu::take_popup_request(ui.ctx(), &salt)).then(|| {
                        Edit::Act(Action::Choose(Pick::Key { scope: scope.clone(), key: key.clone() }))
                    })
                }
                ty => {
                    // a section that is an item shows that item's icon and name
                    let look = (*ty == KeyType::Section)
                        .then(|| {
                            looks
                                .iter()
                                .find(|(s, l)| l.is_some() && first_name(&shown).eq_ignore_ascii_case(s))
                        })
                        .flatten();
                    let extra = if look.is_some() { em(10.0) } else { 0.0 };
                    let w = (ui.available_width() - unit_w(ui) - extra).max(em(4.0));
                    let own_text = match &row.own {
                        Own::Set(v) => Some(v.ltx()),
                        _ => None,
                    };
                    let (typed, f) = local_value_text(ui, &salt, own_text.as_deref(), muted.clone(), w);
                    focus |= f;
                    if !row.unit.is_empty() {
                        let _ = text::label(ui, cat(&row.unit), Font::Body, p.text_muted, Clip::End);
                    }
                    if let Some((section, src)) = look {
                        let (r, _) = ui.allocate_exact_size(THUMB, Sense::hover());
                        let _ = self.paint_icon(ui, r, src.as_ref());
                        let name =
                            self.item_lists.game_item(section).map(|x| x.name.clone()).unwrap_or_default();
                        let _ = text::label(ui, Tx::raw(name), Font::Small, p.text_secondary, Clip::End);
                    }
                    typed.map(|t| {
                        if t.trim().is_empty() {
                            Edit::Reset { scope: scope.clone(), key: key.clone() }
                        } else {
                            Edit::Set {
                                scope: scope.clone(),
                                key: key.clone(),
                                value: item_model::typed(&row.ty, &t),
                            }
                        }
                    })
                }
            }
        });
        let Some(out) = drawn else { return };
        if let Some(e) = out {
            if hud {
                edits.push(Edit::HudRow(row.key.clone()));
            }
            edits.push(e);
        } else if hud && focus {
            edits.push(Edit::HudRow(row.key.clone()));
        }
        if g.was_reset() {
            edits.push(Edit::Reset { scope: row.scope.clone(), key: row.key.clone() });
        }
        // one look at what the row names: played, opened, shown
        let mut looked = false;
        let named = first_name(&shown);
        if !named.is_empty() {
            let act = match row.ty {
                KeyType::Sound => Some((
                    Icon::Play,
                    Tx::from("Play the sound (again to stop)"),
                    Action::Play(named.clone()),
                )),
                KeyType::Visual => Some((
                    Icon::Eye,
                    Tx::from("Open the model in the model viewer"),
                    Action::OpenModel(named.clone()),
                )),
                KeyType::Texture => Some((
                    Icon::Picture,
                    Tx::from("Open the texture in the texture preview"),
                    Action::OpenTexture(named.clone()),
                )),
                _ => None,
            };
            // a section that is an item is picked among the items, by their icons
            let act = act.or_else(|| {
                (row.ty == KeyType::Section && !row.list && self.item_lists.game_item(&named).is_some()).then(
                    || {
                        (
                            Icon::Item,
                            Tx::from("Pick another item"),
                            Action::Choose(Pick::Key { scope: row.scope.clone(), key: row.key.clone() }),
                        )
                    },
                )
            });
            if let Some((icon, tip, action)) = act {
                looked = true;
                if g.trailing(icon, tip) {
                    edits.push(Edit::Act(action));
                }
            }
        }
        // a change can take an inherited key away; one icon at a row's end, so not beside a look
        if change
            && !looked
            && !own
            && row.resolved.is_some()
            && !row.required
            && g.trailing(Icon::Clear, "Take the key away from the item")
        {
            edits.push(Edit::Unset { scope: row.scope.clone(), key: row.key.clone() });
        }
        self.messages(g, &place);
    }

    /// A member of a list as a chip: the player's name of an item beside its section.
    fn member_label(&self, member: &str) -> Tx<'static> {
        match self.item_lists.game_item(member).map(|g| g.name.clone()).filter(|n| !n.is_empty()) {
            Some(n) => Tx::raw(format!("{n}  ({member})")),
            None => Tx::raw(member.to_owned()),
        }
    }

    // ---- Where It Is Found ----------------------------------------------------------------------

    /// Who sells the item, who carries it and what makes it: facts the module's traders, loot
    /// tables and recipes say, each in its own file. The item says none of it - a row opens the
    /// file that does.
    fn found_section(&self, g: &mut Grid<'_>, edits: &mut Vec<Edit>) {
        let section = self.edit.game_section(&self.module);
        let found = self.loaded.as_ref().map(|l| l.roster.found(&section)).unwrap_or_default();
        let badge = (!found.is_empty()).then(|| Tx::raw(found.len().to_string()));
        g.section("Where It Is Found", !found.is_empty(), badge, |g| {
            for f in &found {
                let (label, help, who) = match f.how {
                    "sold" => (
                        "Sold By",
                        "A trader of the game stocks it: the trader's file says how many and how often.",
                        self.trader_label(&f.at),
                    ),
                    "carried" => (
                        "Carried by the Dead",
                        "A loot table says how many the dead carry and the chance, for everybody and by community.",
                        Tx::EMPTY,
                    ),
                    "made" => (
                        "Made At",
                        "A recipe makes it at this tool or workbench, from the parts named.",
                        self.member_label(&f.at),
                    ),
                    _ => ("Part Of", "A recipe takes it as a part of what it makes.", self.member_label(&f.at)),
                };
                g.row(label, help, PropFlags::NONE, 0, |ui| {
                    let p = palette(ui.ctx());
                    let detail = if f.detail == gz_items::roster::OFF_SHELF {
                        Tx::from("Taken off the shelf").text(ui.ctx())
                    } else {
                        f.detail.clone()
                    };
                    let who = who.text(ui.ctx());
                    let line = match (who.is_empty(), detail.is_empty()) {
                        (false, false) => format!("{who}  -  {detail}"),
                        (false, true) => who,
                        _ => detail,
                    };
                    let _ = text::label(ui, Tx::raw(line), Font::Body, p.text, Clip::End)
                        .tip(Tx::raw(f.file.clone()));
                });
                if g.trailing(Icon::Open, "Open the file that says it") {
                    edits.push(Edit::Act(Action::OpenEconomy(f.file.clone())));
                }
            }
            if found.is_empty() {
                g.message(
                    Tone::Muted,
                    "No trader, loot table or recipe of the module names this item. Each says it in a file of its own.",
                );
            }
            g.block(|ui| {
                let open = gz_ui::Button::new("Open Economy")
                    .icon(Icon::Trader)
                    .tip("The module's traders, loot tables and recipes: who sells an item, who carries it and what makes it")
                    .show(ui);
                if open.clicked() {
                    edits.push(Edit::Act(Action::OpenEconomy(String::new())));
                }
            });
        });
    }

    /// A trader as a fact names it: the name the list of traders gives its config.
    fn trader_label(&self, trader: &str) -> Tx<'static> {
        match self.item_lists.traders.get(trader).filter(|e| !e.name.is_empty()) {
            Some(e) => crate::lists::Indexed::label_tx(e),
            None => Tx::raw(trader.to_owned()),
        }
    }

    // ---- Use Scene ------------------------------------------------------------------------------

    fn scene_section(&self, g: &mut Grid<'_>, form: &item_model::Form, edits: &mut Vec<Edit>) {
        let used = form.family.as_ref().is_some_and(|f| self.item_lists.scene_families.contains(f));
        if self.edit.scene.is_none() && !used {
            return;
        }
        let (like, keys) = match &self.edit.scene {
            Some(Scene::Like(s)) => (s.clone(), Vec::new()),
            Some(Scene::Table { like, keys, .. }) => (like.clone().unwrap_or_default(), keys.clone()),
            None => (String::new(), Vec::new()),
        };
        let fit = keys.iter().find(|(k, _)| k == "fit").map(|(_, v)| v.ltx()).unwrap_or_default();
        g.section("Use Scene", self.edit.scene.is_some(), None, |g| {
            let mut v = like.clone();
            if g.row(
                "Like",
                "The item whose use scene this one takes: its animation, sound and camera.",
                PropFlags::NONE,
                0,
                |ui| {
                    list_picker(
                        ui,
                        "item.scene.like",
                        &mut v,
                        &self.item_lists.scenes,
                        &self.item_lists.scenes,
                        Some(Tx::from("None")),
                        !self.item_lists.loaded,
                    )
                },
            ) == Some(true)
            {
                edits.push(Edit::Item(Box::new(move |it| set_scene(it, Some(v), None))));
            }
            self.messages(g, &Place::Part { name: "scene".into(), row: None });
            g.section("More", !keys.is_empty(), None, |g| {
                for (key, label, help) in [
                    ("sound", "Sound", "The scene's sound."),
                    ("camera", "Camera", "The camera effect."),
                    ("time", "Time", "How long the scene lasts, milliseconds."),
                    ("helmet", "Helmet", "What a helmet does to it, as the game's scenes say it."),
                ] {
                    let own = keys.iter().find(|(k, _)| k == key).map(|(_, v)| v.ltx());
                    if let Some((Some(typed), _)) = g.row(label, help, PropFlags::NONE, 0, |ui| {
                        // the time is in the game's milliseconds (`tm = 7200`): the unit after it,
                        // as a catalog row shows its own
                        let unit = (key == "time").then_some("ms");
                        let unit_w =
                            unit.map_or(0.0, |u| text::width(ui.ctx(), &Font::Body.id(), Tx::from(u)) + 8.0);
                        let typed = local_value_text(
                            ui,
                            &format!("item.scene.{key}"),
                            own.as_deref(),
                            Tx::EMPTY,
                            (ui.available_width() - unit_w).max(em(4.0)),
                        );
                        if let Some(u) = unit {
                            let _ = text::label(ui, u, Font::Body, palette(ui.ctx()).text_muted, Clip::End);
                        }
                        typed
                    }) {
                        let value =
                            (!typed.trim().is_empty()).then(|| match gz_ui::grid::parse_number(&typed) {
                                Some(n) if key == "time" => SetValue::Number(n),
                                _ => SetValue::Text(typed.trim().to_owned()),
                            });
                        edits.push(Edit::Item(Box::new(move |it| set_scene(it, None, Some((key, value))))));
                    }
                }
                // how a hands motion longer than the scene fits it: one of two words
                let cut = fit.eq_ignore_ascii_case("cut");
                let words = [Tx::from(FIT_HURRY), Tx::from(FIT_CUT)];
                if let Some(Some(picked)) = g.row(
                    "Long Motion",
                    "A hands motion longer than the scene: hurried to fit it, or kept at its own pace and cut where the scene ends.",
                    PropFlags::NONE,
                    0,
                    |ui| {
                        crate::picker::row_combo(
                            ui,
                            "item.scene.fit",
                            Tx::from(if cut { FIT_CUT } else { FIT_HURRY }),
                            false,
                            &words,
                            Some(usize::from(cut)),
                        )
                    },
                ) && (picked == 1) != cut
                {
                    let value = (picked == 1).then(|| SetValue::Text("cut".to_owned()));
                    edits.push(Edit::Item(Box::new(move |it| set_scene(it, None, Some(("fit", value))))));
                }
            });
        });
    }

    // ---- More Sections, Other Keys --------------------------------------------------------------

    fn sections_section(&self, g: &mut Grid<'_>, edits: &mut Vec<Edit>) {
        for (si, s) in self.edit.sections.iter().enumerate() {
            g.block(|ui| {
                let p = palette(ui.ctx());
                let title = if s.name.is_empty() {
                    tr!("Changes {section}", section = s.edit.as_str())
                } else {
                    Tx::raw(s.name.clone())
                };
                let _ = text::label(ui, title, Font::Strong, p.text, Clip::End);
                if !s.base.is_empty() {
                    let _ = text::label(
                        ui,
                        tr!("Starts from {base}", base = s.base.join(", ")),
                        Font::Small,
                        p.text_secondary,
                        Clip::End,
                    );
                }
                let mut set: Option<(usize, usize, String)> = None;
                let columns = [Col::new("Key", em(12.0)), Col::new("Value", em(14.0))];
                let rows = &s.changes.set;
                let result = ItemRows::new(("item.sections", si), &columns, rows.len())
                    .add_label("Add Key")
                    .empty_text("It sets no key.")
                    .show(ui, |ui, row, col| {
                        let Some((k, v)) = rows.get(row) else { return };
                        let shown = if col == 0 { k.clone() } else { v.ltx() };
                        if let (Some(t), _) = local_value_text(
                            ui,
                            &format!("item.sections.{si}.{row}.{col}"),
                            Some(&shown),
                            Tx::EMPTY,
                            ui.available_width(),
                        ) {
                            set = Some((row, col, t));
                        }
                    });
                if let Some((r, col, t)) = set {
                    edits.push(Edit::Item(Box::new(move |it| {
                        edit_pair(&mut section_at(it, si).changes.set, r, col, t)
                    })));
                }
                if let Some(r) = result.removed {
                    edits.push(Edit::Item(Box::new(move |it| {
                        let set = &mut section_at(it, si).changes.set;
                        if r < set.len() {
                            set.remove(r);
                        }
                    })));
                }
                if result.added {
                    edits.push(Edit::Item(Box::new(move |it| {
                        section_at(it, si).changes.set.push(("new_key".into(), SetValue::Text(String::new())))
                    })));
                }
                // what a change takes away and what it does to a list, edited as rows too
                let changes_section = !s.edit.is_empty();
                if changes_section || !s.changes.unset.is_empty() {
                    ui.add_space(6.0);
                    let _ = text::label(ui, "Takes Away", Font::Small, p.text_secondary, Clip::End);
                    self.unset_rows(ui, si, &s.changes.unset, edits);
                }
                let mut lists: Vec<String> = Vec::new();
                for (k, _) in s.changes.add.iter().chain(s.changes.drop.iter()) {
                    if !lists.contains(k) {
                        lists.push(k.clone());
                    }
                }
                if changes_section || !lists.is_empty() {
                    ui.add_space(6.0);
                    let _ = text::label(ui, "Changes a List", Font::Small, p.text_secondary, Clip::End);
                    self.list_rows(ui, si, s, &lists, edits);
                }
            });
            self.messages(g, &Place::Part { name: "sections".into(), row: Some(si) });
        }
    }

    /// The rows Key | Value of what a scope's section sets that the catalog lacks: the item's own
    /// section or its HUD section's.
    /// The keys a row of More Sections takes away: a row each, typed; the cross brings one back.
    fn unset_rows(&self, ui: &mut Ui, si: usize, rows: &[String], edits: &mut Vec<Edit>) {
        let columns = [Col::new("Key", em(12.0))];
        let mut typed: Option<(usize, String)> = None;
        let result = ItemRows::new(("item.sections.unset", si), &columns, rows.len())
            .add_label("Take a Key Away")
            .empty_text("It takes no key away.")
            .show(ui, |ui, row, _| {
                let Some(k) = rows.get(row) else { return };
                if let (Some(t), _) = local_value_text(
                    ui,
                    &format!("item.sections.{si}.unset.{row}"),
                    Some(k),
                    Tx::EMPTY,
                    ui.available_width(),
                ) {
                    typed = Some((row, t));
                }
            });
        if let Some((r, t)) = typed {
            let t = t.trim().to_owned();
            if gz_items::model::plain_name(&t) {
                edits.push(Edit::Item(Box::new(move |it| {
                    if let Some(k) = section_at(it, si).changes.unset.get_mut(r) {
                        *k = t;
                    }
                })));
            }
        }
        if let Some(r) = result.removed {
            edits.push(Edit::Item(Box::new(move |it| {
                let unset = &mut section_at(it, si).changes.unset;
                if r < unset.len() {
                    unset.remove(r);
                }
            })));
        }
        if result.added {
            edits.push(Edit::Item(Box::new(move |it| {
                let unset = &mut section_at(it, si).changes.unset;
                let mut name = "new_key".to_owned();
                let mut n = 2;
                while unset.contains(&name) {
                    name = format!("new_key_{n}");
                    n += 1;
                }
                unset.push(name);
            })));
        }
    }

    /// What a row of More Sections does to a list: Key | Adds | Takes Out, the members as a comma
    /// list; a list without members goes when the item is saved.
    fn list_rows(&self, ui: &mut Ui, si: usize, s: &OwnedSection, lists: &[String], edits: &mut Vec<Edit>) {
        let columns =
            [Col::new("Key", em(10.0)), Col::new("Adds", em(10.0)), Col::new("Takes Out", em(10.0))];
        let members = |list: &[(String, Vec<String>)], key: &str| -> String {
            list.iter()
                .filter(|(k, _)| k == key)
                .flat_map(|(_, m)| m.iter().cloned())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut typed: Option<(String, usize, String)> = None;
        let result = ItemRows::new(("item.sections.lists", si), &columns, lists.len())
            .add_label("Add a List Change")
            .empty_text("It changes no list.")
            .show(ui, |ui, row, col| {
                let Some(key) = lists.get(row) else { return };
                let shown = match col {
                    0 => key.clone(),
                    1 => members(&s.changes.add, key),
                    _ => members(&s.changes.drop, key),
                };
                if let (Some(t), _) = local_value_text(
                    ui,
                    &format!("item.sections.{si}.list.{row}.{col}"),
                    Some(&shown),
                    Tx::EMPTY,
                    ui.available_width(),
                ) {
                    typed = Some((key.clone(), col, t));
                }
            });
        if let Some((key, col, t)) = typed {
            edits.push(Edit::Item(Box::new(move |it| {
                let c = &mut section_at(it, si).changes;
                let t = t.trim().to_owned();
                match col {
                    0 => {
                        if gz_items::model::plain_name(&t) {
                            for (k, _) in c.add.iter_mut().chain(c.drop.iter_mut()) {
                                if *k == key {
                                    k.clone_from(&t);
                                }
                            }
                        }
                    }
                    _ => {
                        let list = if col == 1 { &mut c.add } else { &mut c.drop };
                        let now = item_model::members_of(&t);
                        match list.iter_mut().find(|(k, _)| *k == key) {
                            Some(slot) => slot.1 = now,
                            None => list.push((key.clone(), now)),
                        }
                    }
                }
            })));
        }
        if let Some(r) = result.removed
            && let Some(key) = lists.get(r).cloned()
        {
            edits.push(Edit::Item(Box::new(move |it| {
                let c = &mut section_at(it, si).changes;
                c.add.retain(|(k, _)| *k != key);
                c.drop.retain(|(k, _)| *k != key);
            })));
        }
        if result.added {
            edits.push(Edit::Item(Box::new(move |it| {
                let c = &mut section_at(it, si).changes;
                let mut name = "new_list".to_owned();
                let mut n = 2;
                while c.add.iter().chain(c.drop.iter()).any(|(k, _)| *k == name) {
                    name = format!("new_list_{n}");
                    n += 1;
                }
                c.add.push((name, Vec::new()));
            })));
        }
    }

    fn other_section(
        &self,
        g: &mut Grid<'_>,
        other: &[OtherRow],
        scope: &'static str,
        edits: &mut Vec<Edit>,
    ) {
        g.block(|ui| {
            let _ = text::label(
                ui,
                if scope == gz_items::catalog::HUD {
                    "What the HUD section sets that the linked game's catalog does not know: written as it is."
                } else {
                    "What the item sets that the linked game's catalog does not know: written as it is."
                },
                Font::Small,
                palette(ui.ctx()).text_secondary,
                Clip::End,
            );
            let rows: Vec<&OtherRow> = other.iter().filter(|o| o.scope == scope).collect();
            let columns = [Col::new("Key", em(12.0)), Col::new("Value", em(14.0))];
            let mut set: Option<(String, usize, String)> = None;
            let result = ItemRows::new(("item.other", scope), &columns, rows.len())
                .add_label("Add Key")
                .empty_text("Nothing the catalog does not know.")
                .show(ui, |ui, row, col| {
                    let Some(o) = rows.get(row) else { return };
                    let value = match &o.own {
                        Own::Set(v) => v.ltx(),
                        Own::Unset => String::new(),
                        Own::Members { add, .. } => add.join(", "),
                        Own::Inherit => String::new(),
                    };
                    let shown = if col == 0 { o.key.clone() } else { value };
                    if let (Some(t), _) = local_value_text(
                        ui,
                        &format!("item.other.{scope}.{}.{col}", o.key),
                        Some(&shown),
                        Tx::EMPTY,
                        ui.available_width(),
                    ) {
                        set = Some((o.key.clone(), col, t));
                    }
                });
            if let Some((key, col, t)) = set {
                edits.push(Edit::Item(Box::new(move |it| {
                    let changes = changes_of(it, scope);
                    if let Some(at) = changes.set.iter().position(|(k, _)| *k == key) {
                        edit_pair(&mut changes.set, at, col, t);
                    } else if col == 1 && !t.trim().is_empty() {
                        item_model::set_own(changes, &key, SetValue::Text(t.trim().to_owned()));
                    }
                })));
            }
            if let Some(r) = result.removed
                && let Some(o) = rows.get(r)
            {
                let key = o.key.clone();
                edits.push(Edit::Item(Box::new(move |it| item_model::reset(changes_of(it, scope), &key))));
            }
            if result.added {
                edits.push(Edit::Item(Box::new(move |it| {
                    let changes = changes_of(it, scope);
                    let mut name = "new_key".to_owned();
                    let mut n = 2;
                    while changes.set.iter().any(|(k, _)| *k == name) {
                        name = format!("new_key_{n}");
                        n += 1;
                    }
                    changes.set.push((name, SetValue::Text(String::new())));
                })));
            }
        });
        for (pl, tone, line) in &self.places {
            if let Place::Key { scope: s, key } = pl
                && s == scope
                && other.iter().any(|o| o.scope == scope && o.key == *key)
            {
                g.message(*tone, Tx::ui(line.as_str()));
            }
        }
    }

    // ---- the preview ----------------------------------------------------------------------------

    /// The item as the game shows it: the icon at twice its size, the names and the description
    /// the player reads, the weight and the cost; what the player sees in the hands; then its
    /// world model. The column scrolls; `view_h` is how tall it shows.
    pub(crate) fn draw_preview(&mut self, ui: &mut Ui, cx: &mut gz_app::DocCx<'_>, view_h: f32) {
        let p = palette(ui.ctx());
        let full = Rect::from_min_size(ui.max_rect().min, Vec2::new(ui.available_width(), view_h));
        let resolved = |k: &str| -> Option<String> {
            self.form.groups.iter().flat_map(|g| g.rows.iter()).find(|r| r.key == k).and_then(Row::shown)
        };
        let unit = |k: &str| {
            self.form
                .groups
                .iter()
                .flat_map(|g| g.rows.iter())
                .find(|r| r.key == k)
                .map(|r| r.unit.clone())
                .unwrap_or_default()
        };
        let visual = resolved(game_keys::VISUAL).unwrap_or_default();
        let _ = text::label(ui, "In the Game", Font::Strong, p.text, Clip::End);
        ui.add_space(6.0);
        let src = self.edit_icon();
        let cells = src.as_ref().and_then(IconSource::cells).unwrap_or((2, 1));
        let want = Vec2::new(cells.0 as f32 * CELL * 2.0, cells.1 as f32 * CELL * 2.0);
        let s = (full.width() / want.x).min(1.0);
        let (rect, _) = ui.allocate_exact_size(want * s, Sense::hover());
        ui.painter().rect_filled(rect, CornerRadius::same(4), p.inset);
        let _ = self.paint_icon(ui, rect.shrink(4.0), src.as_ref());
        ui.add_space(8.0);
        let pick = |own: &str, hint: &str| if own.is_empty() { hint.to_owned() } else { own.to_owned() };
        let (eng, rus) = (pick(&self.texts[1], &self.hints[1]), pick(&self.texts[0], &self.hints[0]));
        let name = if rus.is_empty() { eng.clone() } else { rus.clone() };
        let _ = text::label(
            ui,
            if name.is_empty() { Tx::from("No name") } else { Tx::raw(name.clone()) },
            Font::Title,
            p.text,
            Clip::End,
        );
        if !eng.is_empty() && eng != name {
            let _ = text::label(ui, Tx::raw(eng), Font::Small, p.text_muted, Clip::End);
        }
        let short = pick(&self.texts[2], &self.hints[2]);
        if !short.is_empty() {
            let _ = text::label(ui, Tx::raw(short), Font::Body, p.text_secondary, Clip::End);
        }
        let descr = pick(&self.texts[4], &self.hints[4]);
        if !descr.is_empty() {
            ui.add_space(4.0);
            let _ = text::prose(ui, Tx::raw(as_read(&descr)));
        }
        ui.add_space(6.0);
        for (key, label) in [(game_keys::WEIGHT, "Weight"), (game_keys::COST, "Cost")] {
            if let Some(v) = resolved(key) {
                let u = unit(key);
                let line = tr!(
                    "{label}: {value} {unit}",
                    label = Tx::from(label),
                    value = v.as_str(),
                    unit = Tx::raw(u)
                );
                let _ = text::label(ui, line, Font::Body, p.text_secondary, Clip::End);
            }
        }
        ui.add_space(8.0);
        // what the player sees in the hands, between the inventory and the world
        self.draw_hands(ui, cx);
        if visual.is_empty() {
            self.model = None;
            self.model_rect = None;
            return;
        }
        let key = first_name(&visual);
        if self.model.as_ref().is_none_or(|(k, _)| *k != key) {
            let req =
                gz_ui_previews::model::open_request(&key, gz_ui_previews::model_load::ModelSource::Game);
            self.model = gz_ui_previews::model::factory(&self.editor, &req).ok().map(|mut d| {
                if let Some(m) =
                    d.as_any_mut().and_then(|a| a.downcast_mut::<gz_ui_previews::model::ModelDoc>())
                {
                    m.show_details = false;
                }
                (key.clone(), d)
            });
        }
        // the world model takes most of what the column shows, never less than a usable view
        let below = ui.cursor().min.y;
        let model_h = (view_h * 0.6).clamp(220.0, 360.0);
        let model_rect = Rect::from_min_size(pos2(full.min.x, below), Vec2::new(full.width(), model_h));
        if let Some((_, doc)) = self.model.as_mut() {
            let mut child =
                ui.new_child(UiBuilder::new().id_salt("gz-ui-world.items.model").max_rect(model_rect));
            doc.body(&mut child, cx);
        }
        self.model_rect = Some(model_rect);
        ui.advance_cursor_after_rect(model_rect);
    }
}

/// A game text as the player reads it: its `\n` a line break, its colour marks
/// (`%c[a,r,g,b]`, `%c[name]`) taken out - the preview draws one colour.
fn as_read(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("\\n") {
            out.push('\n');
            rest = after;
        } else if let Some(after) = rest.strip_prefix("%c[")
            && let Some(end) = after.find(']')
        {
            rest = &after[end + 1..];
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The changes of a scope's section of an item: its own, or its HUD's.
fn changes_of<'a>(it: &'a mut gz_items::Item, scope: &str) -> &'a mut gz_items::Changes {
    if scope == gz_items::catalog::HUD {
        &mut it.hud.get_or_insert_with(gz_items::Hud::default).changes
    } else {
        &mut it.changes
    }
}

/// The two ways a long hands motion fits its scene, as the Use Scene's row offers them.
const FIT_HURRY: &str = "Hurried";
const FIT_CUT: &str = "Cut Short";

fn section_at(it: &mut gz_items::Item, i: usize) -> &mut OwnedSection {
    if i >= it.sections.len() {
        it.sections.resize_with(i + 1, OwnedSection::default);
    }
    &mut it.sections[i]
}

/// A key or a value of a `key = value` row typed.
fn edit_pair(set: &mut [(String, SetValue)], row: usize, col: usize, typed: String) {
    let Some(pair) = set.get_mut(row) else { return };
    let t = typed.trim().to_owned();
    if col == 0 {
        if gz_items::model::plain_name(&t) {
            pair.0 = t;
        }
    } else {
        pair.1 = match gz_ui::grid::parse_number(&t) {
            Some(n) if n.to_string() == t || gz_luadata::number_to_text(n) == t => SetValue::Number(n),
            _ => SetValue::Text(t),
        };
    }
}

/// The use scene with `like` (when given) and a key set or taken away.
fn set_scene(it: &mut gz_items::Item, like: Option<String>, key: Option<(&'static str, Option<SetValue>)>) {
    let (mut l, mut keys, hud, set) = match it.scene.take() {
        Some(Scene::Like(s)) => (Some(s), Vec::new(), None, Vec::new()),
        Some(Scene::Table { like, keys, hud, set }) => (like, keys, hud, set),
        None => (None, Vec::new(), None, Vec::new()),
    };
    if let Some(v) = like {
        l = (!v.is_empty()).then_some(v);
    }
    if let Some((k, v)) = key {
        keys.retain(|(x, _)| x != k);
        if let Some(v) = v {
            keys.push((k.to_owned(), v));
        }
    }
    it.scene = match (l, keys.is_empty() && hud.is_none() && set.is_empty()) {
        (None, true) => None,
        (Some(s), true) => Some(Scene::Like(s)),
        (like, _) => Some(Scene::Table { like, keys, hud, set }),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_scene_is_as_short_as_what_it_says() {
        let mut it = gz_items::Item::default();
        set_scene(&mut it, Some("vodka".into()), None);
        assert_eq!(it.scene, Some(Scene::Like("vodka".into())));
        set_scene(&mut it, None, Some(("time", Some(SetValue::Number(3.0)))));
        assert!(
            matches!(&it.scene, Some(Scene::Table { like: Some(l), keys, .. }) if l == "vodka" && keys.len() == 1)
        );
        set_scene(&mut it, Some(String::new()), Some(("time", None)));
        assert_eq!(it.scene, None);
    }

    #[test]
    fn a_game_text_reads_as_the_player_reads_it() {
        assert_eq!(as_read("A\\n%c[0,140,140,140]B \u{2022} %c[ui_gray_1]C"), "A\nB \u{2022} C");
        assert_eq!(as_read("100%c"), "100%c");
    }

    #[test]
    fn a_pair_typed_keeps_a_key_a_config_can_hold() {
        // a scene keeps the keys of its own section through an edit of another key
        let mut it = gz_items::Item {
            scene: Some(Scene::Table {
                like: None,
                keys: Vec::new(),
                hud: None,
                set: vec![("my_flag".to_owned(), SetValue::Bool(true))],
            }),
            ..gz_items::Item::default()
        };
        set_scene(&mut it, None, Some(("fit", Some(SetValue::Text("cut".into())))));
        set_scene(&mut it, None, Some(("fit", None)));
        assert!(
            matches!(&it.scene, Some(Scene::Table { set, keys, .. }) if set.len() == 1 && keys.is_empty())
        );
        let mut set = vec![("a".to_owned(), SetValue::Text("x".into()))];
        edit_pair(&mut set, 0, 1, "12".into());
        assert_eq!(set[0].1, SetValue::Number(12.0));
        edit_pair(&mut set, 0, 0, "bad key".into());
        assert_eq!(set[0].0, "a");
    }
}
