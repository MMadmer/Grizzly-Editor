//! [`ItemRows`] - a small editable table, and [`Table`] - a list of records (docs/UI_DESIGN.md,
//! section 4).
//!
//! - **`ItemRows`**: named columns, the caller's widgets in the cells, a cross per row, an Add
//!   row under them, a grip to drag a row to another place. A starting kit, a trader's stock, a
//!   list of layers. It answers what the author did; the caller changes its data.
//! - **`Table`**: a header of sortable, resizable columns (their widths kept in the layout store
//!   by the table's key), virtualized rows, a selection, and a context menu slot. A list of
//!   releases, of commands.

use egui::{Align2, CornerRadius, CursorIcon, Id, Rect, Response, Sense, Stroke, Ui, UiBuilder, Vec2, pos2};

use crate::audit;
use crate::button::icon_button_sized;
use crate::font::Font;
use crate::icons::{self, Icon};
use crate::layout;
use crate::text::{self, Clip};
use crate::theme::{metrics, palette};

/// A column of an [`ItemRows`] or a [`Table`].
#[derive(Clone, Copy, Debug)]
pub struct Col<'a> {
    pub name: &'a str,
    /// Its width at first, in points; the last column of an `ItemRows` takes what is left.
    pub width: f32,
    /// The narrowest a drag makes it.
    pub min: f32,
    /// A click on its name sorts by it (a `Table`).
    pub sortable: bool,
    /// Centred over its cells (a check column).
    pub centred: bool,
}

impl<'a> Col<'a> {
    #[must_use]
    pub const fn new(name: &'a str, width: f32) -> Self {
        Self { name, width, min: 32.0, sortable: false, centred: false }
    }

    #[must_use]
    pub const fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }

    #[must_use]
    pub const fn centred(mut self) -> Self {
        self.centred = true;
        self
    }

    #[must_use]
    pub const fn min(mut self, min: f32) -> Self {
        self.min = min;
        self
    }
}

/// What the author did to an [`ItemRows`] this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowsEdit {
    /// The cross of this row was pressed.
    pub removed: Option<usize>,
    /// The Add row was pressed.
    pub added: bool,
    /// A row was dropped at another place: from, to (the index it takes).
    pub moved: Option<(usize, usize)>,
}

const ROW_H: f32 = metrics::PROPERTY_ROW + 4.0;
const HEAD_H: f32 = 24.0;
const GRIP_W: f32 = 18.0;
const CROSS_W: f32 = metrics::ICON_BUTTON + 4.0;

#[derive(Clone, Copy, Debug, Default)]
struct Drag {
    from: usize,
    /// The pointer's y when the drag began, relative to the row's top.
    grab: f32,
}

/// An editable table of a few rows.
#[must_use = "item rows do nothing until they are shown"]
pub struct ItemRows<'a> {
    id: Id,
    columns: &'a [Col<'a>],
    rows: usize,
    add_label: &'a str,
    reorder: bool,
    empty: &'a str,
}

impl<'a> ItemRows<'a> {
    pub fn new(id_salt: impl std::hash::Hash + std::fmt::Debug, columns: &'a [Col<'a>], rows: usize) -> Self {
        Self {
            id: Id::unique(("gz-ui.item_rows", id_salt)),
            columns,
            rows,
            add_label: "Add",
            reorder: true,
            empty: "",
        }
    }

    /// The Add row's label: "Add Item".
    pub fn add_label(mut self, label: &'a str) -> Self {
        self.add_label = label;
        self
    }

    /// The rows keep their order: no grip.
    pub fn fixed_order(mut self) -> Self {
        self.reorder = false;
        self
    }

    /// What it says while it has no rows: "Nothing in the kit yet."
    pub fn empty_text(mut self, text_: &'a str) -> Self {
        self.empty = text_;
        self
    }

    /// Draws the header, the rows (each cell is `cell(ui, row, column)` in a child as wide as the
    /// column) and the Add row. Returns what the author did.
    pub fn show(self, ui: &mut Ui, mut cell: impl FnMut(&mut Ui, usize, usize)) -> RowsEdit {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let mut edit = RowsEdit::default();
        let width = ui.available_width();
        let lead = if self.reorder { GRIP_W } else { 0.0 };
        let fixed: f32 =
            self.columns.iter().take(self.columns.len().saturating_sub(1)).map(|c| c.width).sum();
        let last = (width - lead - CROSS_W - fixed).max(self.columns.last().map_or(0.0, |c| c.min));
        let widths: Vec<f32> = self
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| if i + 1 == self.columns.len() { last } else { c.width })
            .collect();

        // the header
        let (head, _) = ui.allocate_exact_size(Vec2::new(width, HEAD_H), Sense::hover());
        let mut x = head.min.x + lead;
        for (c, w) in self.columns.iter().zip(&widths) {
            let tx = if c.centred { x + w * 0.5 } else { x + 6.0 };
            text::paint(
                ui.painter(),
                &Font::Small.id(),
                pos2(tx, head.center().y),
                if c.centred { Align2::CENTER_CENTER } else { Align2::LEFT_CENTER },
                p.text_secondary,
                c.name,
                Some((w - 8.0).max(0.0)),
                Clip::End,
            );
            x += w;
        }
        ui.painter().line_segment(
            [pos2(head.min.x, head.max.y - 0.5), pos2(head.max.x, head.max.y - 0.5)],
            Stroke::new(1.0, p.border_subtle),
        );

        let drag_id = self.id.with("drag");
        let mut drag = ctx.data(|d| d.get_temp::<Drag>(drag_id));
        let top = ui.cursor().min.y;
        if self.rows == 0 && !self.empty.is_empty() {
            let (r, _) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::hover());
            ui.painter().text(
                pos2(r.min.x + lead + 6.0, r.center().y),
                Align2::LEFT_CENTER,
                self.empty,
                Font::Body.id(),
                p.text_muted,
            );
        }
        let pointer = ctx.pointer_latest_pos();
        let target = drag.and_then(|d| {
            pointer.map(|at| {
                (((at.y - d.grab - top) / ROW_H).round().max(0.0) as usize).min(self.rows.saturating_sub(1))
            })
        });
        for row in 0..self.rows {
            let (r, _) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::hover());
            let dragged = drag.is_some_and(|d| d.from == row);
            if dragged {
                ui.painter().rect_filled(r, CornerRadius::same(metrics::RADIUS), p.hover);
            }
            if target.is_some_and(|t| t == row) && drag.is_some_and(|d| d.from != row) {
                let y = if drag.is_some_and(|d| d.from < row) { r.max.y } else { r.min.y };
                ui.painter()
                    .line_segment([pos2(r.min.x, y), pos2(r.max.x, y)], Stroke::new(2.0, p.accent_bright));
            }
            if self.reorder {
                let grip = Rect::from_min_size(pos2(r.min.x, r.min.y), Vec2::new(GRIP_W, ROW_H));
                let gr = ui.interact(grip, self.id.with(("grip", row)), Sense::drag());
                if gr.hovered() || gr.dragged() {
                    ctx.set_cursor_icon(CursorIcon::Grab);
                }
                icons::draw(
                    ui.painter(),
                    Icon::Grip,
                    Rect::from_center_size(grip.center(), Vec2::splat(metrics::GLYPH_STATUS)),
                    if gr.hovered() || dragged { p.text } else { p.text_muted },
                );
                if gr.drag_started() {
                    let grab = pointer.map_or(ROW_H * 0.5, |at| at.y - r.min.y);
                    drag = Some(Drag { from: row, grab });
                }
            }
            let mut x = r.min.x + lead;
            for (col, w) in widths.iter().enumerate() {
                let cell_rect = Rect::from_min_size(pos2(x, r.min.y + 2.0), Vec2::new(w - 4.0, ROW_H - 4.0));
                let mut child = ui.new_child(
                    UiBuilder::new()
                        .id_salt((self.id, row, col))
                        .max_rect(cell_rect)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                );
                cell(&mut child, row, col);
                x += w;
            }
            let cross = Rect::from_min_size(
                pos2(r.max.x - CROSS_W + 2.0, r.center().y - metrics::ICON_BUTTON * 0.5),
                Vec2::splat(metrics::ICON_BUTTON),
            );
            let mut c = ui.new_child(UiBuilder::new().id_salt((self.id, row, "x")).max_rect(cross));
            if icon_button_sized(
                &mut c,
                Icon::Close,
                "Remove this row",
                false,
                true,
                metrics::ICON_BUTTON,
                12.0,
            )
            .clicked()
            {
                edit.removed = Some(row);
            }
        }
        if let Some(d) = drag {
            if ctx.input(|i| i.pointer.any_released()) {
                if let Some(t) = target
                    && t != d.from
                {
                    edit.moved = Some((d.from, t));
                }
                ctx.data_mut(|m| m.remove::<Drag>(drag_id));
            } else {
                ctx.data_mut(|m| m.insert_temp(drag_id, d));
            }
        }
        // the Add row
        let (r, _) = ui.allocate_exact_size(Vec2::new(width, ROW_H), Sense::hover());
        let add_rect = Rect::from_min_max(pos2(r.min.x + lead, r.min.y + 2.0), pos2(r.max.x, r.max.y - 2.0));
        let resp = ui.interact(add_rect, self.id.with("add"), Sense::click());
        if resp.hovered() {
            ui.painter().rect_filled(add_rect, CornerRadius::same(metrics::RADIUS), p.hover);
        }
        icons::draw(
            ui.painter(),
            Icon::Plus,
            Rect::from_min_size(pos2(add_rect.min.x + 6.0, add_rect.center().y - 7.0), Vec2::splat(14.0)),
            if resp.hovered() { p.text } else { p.text_secondary },
        );
        ui.painter().text(
            pos2(add_rect.min.x + 26.0, add_rect.center().y),
            Align2::LEFT_CENTER,
            self.add_label,
            Font::Body.id(),
            if resp.hovered() { p.text } else { p.text_secondary },
        );
        audit::control(&ctx, self.add_label, true, None, None, add_rect);
        if resp.clicked() {
            edit.added = true;
        }
        edit
    }
}

/// Moves `items[from]` to `to`, as [`RowsEdit::moved`] says.
pub fn apply_move<T>(items: &mut Vec<T>, from: usize, to: usize) {
    if from < items.len() && to < items.len() && from != to {
        let it = items.remove(from);
        items.insert(to, it);
    }
}

/// What a [`Table`] answered this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TableEvent {
    /// A row was clicked (selected).
    pub clicked: Option<usize>,
    pub double_clicked: Option<usize>,
    /// A row was right-clicked: the caller opens its context menu (the row is selected first).
    pub context: Option<usize>,
    /// The header asked for another sort: the column, ascending.
    pub sort: Option<(usize, bool)>,
}

/// The rows of a record list.
#[must_use = "a table does nothing until it is shown"]
pub struct Table<'a> {
    key: &'a str,
    columns: &'a [Col<'a>],
    rows: usize,
    selected: Option<usize>,
    sorted: Option<(usize, bool)>,
    row_h: f32,
    height: Option<f32>,
}

impl<'a> Table<'a> {
    /// `key` keeps the column widths in the layout store (`table.<key>.<n>`).
    pub fn new(key: &'a str, columns: &'a [Col<'a>], rows: usize) -> Self {
        Self { key, columns, rows, selected: None, sorted: None, row_h: metrics::ROW + 4.0, height: None }
    }

    pub fn selected(mut self, row: Option<usize>) -> Self {
        self.selected = row;
        self
    }

    /// Sorted by this column, ascending or not: the header shows it.
    pub fn sorted(mut self, column: usize, ascending: bool) -> Self {
        self.sorted = Some((column, ascending));
        self
    }

    pub fn row_height(mut self, h: f32) -> Self {
        self.row_h = h;
        self
    }

    /// The height of the rows' area; left out, what is left.
    pub fn height(mut self, h: f32) -> Self {
        self.height = Some(h);
        self
    }

    fn width_key(&self, col: usize) -> String {
        format!("table.{}.{col}", self.key)
    }

    /// The widths of the columns now: what the author dragged, else their first widths; the last
    /// takes what is left of `width`.
    fn widths(&self, ui: &Ui, width: f32) -> Vec<f32> {
        let n = self.columns.len();
        let mut out: Vec<f32> = self
            .columns
            .iter()
            .enumerate()
            .map(|(i, c)| layout::get(ui.ctx(), &self.width_key(i), c.width).max(c.min))
            .collect();
        if let Some(last) = out.last_mut() {
            let rest: f32 = self
                .columns
                .iter()
                .take(n - 1)
                .enumerate()
                .map(|(i, _)| {
                    layout::get(ui.ctx(), &self.width_key(i), self.columns[i].width).max(self.columns[i].min)
                })
                .sum();
            *last = (width - rest).max(self.columns[n - 1].min);
        }
        out
    }

    /// Draws the header and the rows in view (`cell(ui, row, column)` for each cell).
    pub fn show(self, ui: &mut Ui, mut cell: impl FnMut(&mut Ui, usize, usize)) -> TableEvent {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let mut ev = TableEvent::default();
        let width = ui.available_width();
        let widths = self.widths(ui, width);

        // the header: names, the sort mark, a line between columns to drag
        let (head, _) = ui.allocate_exact_size(Vec2::new(width, HEAD_H), Sense::hover());
        let mut x = head.min.x;
        for (i, (c, w)) in self.columns.iter().zip(&widths).enumerate() {
            let rect = Rect::from_min_size(pos2(x, head.min.y), Vec2::new(*w, HEAD_H));
            let sorted = self.sorted.filter(|(col, _)| *col == i).map(|(_, asc)| asc);
            let resp = ui.interact(
                rect,
                Id::unique(("gz-ui.table", self.key, "head", i)),
                if c.sortable { Sense::click() } else { Sense::hover() },
            );
            let color = if c.sortable && resp.hovered() { p.text } else { p.text_secondary };
            let sort_w = if sorted.is_some() { metrics::GLYPH_STATUS + 4.0 } else { 0.0 };
            let room = (w - 12.0 - sort_w).max(0.0);
            let name_w = text::width(&ctx, &Font::Small.id(), c.name).min(room);
            let tx = if c.centred { rect.center().x - (name_w + sort_w) * 0.5 } else { rect.min.x + 6.0 };
            text::paint(
                ui.painter(),
                &Font::Small.id(),
                pos2(tx, rect.center().y),
                Align2::LEFT_CENTER,
                color,
                c.name,
                Some(room),
                Clip::End,
            );
            if let Some(asc) = sorted {
                icons::draw(
                    ui.painter(),
                    if asc { Icon::SortAsc } else { Icon::SortDesc },
                    Rect::from_min_size(
                        pos2(tx + name_w + 4.0, rect.center().y - 6.0),
                        Vec2::splat(metrics::GLYPH_STATUS),
                    ),
                    p.text_secondary,
                );
            }
            if c.sortable && resp.clicked() {
                ev.sort = Some((i, sorted.is_none_or(|asc| !asc)));
            }
            if c.sortable {
                audit::control(&ctx, c.name, true, None, Some("Sort by this column"), rect);
            }
            // a column is resized from its right edge (not the last one: it takes what is left)
            if i + 1 < self.columns.len() {
                let edge = Rect::from_min_max(
                    pos2(rect.max.x - 3.0, rect.min.y),
                    pos2(rect.max.x + 3.0, rect.max.y),
                );
                let er = ui.interact(
                    edge,
                    Id::unique(("gz-ui.table", self.key, "edge", i)),
                    Sense::click_and_drag(),
                );
                if er.hovered() || er.dragged() {
                    ctx.set_cursor_icon(CursorIcon::ResizeColumn);
                }
                ui.painter().line_segment(
                    [pos2(rect.max.x - 0.5, rect.min.y + 5.0), pos2(rect.max.x - 0.5, rect.max.y - 5.0)],
                    Stroke::new(
                        1.0,
                        if er.hovered() || er.dragged() { p.border_strong } else { p.border_subtle },
                    ),
                );
                if er.dragged() {
                    let next = (w + er.drag_delta().x).max(c.min);
                    layout::set(&ctx, &self.width_key(i), next);
                }
                if er.double_clicked() {
                    layout::reset(&ctx, &self.width_key(i));
                }
            }
            x += w;
        }
        ui.painter().line_segment(
            [pos2(head.min.x, head.max.y - 0.5), pos2(head.max.x, head.max.y - 0.5)],
            Stroke::new(1.0, p.border_subtle),
        );

        let height = self.height.unwrap_or_else(|| ui.available_height()).max(self.row_h);
        egui::ScrollArea::vertical()
            .id_salt(("gz-ui.table", self.key))
            .max_height(height)
            .auto_shrink([false, false])
            .show_rows(ui, self.row_h, self.rows, |ui, range| {
                ui.spacing_mut().item_spacing = Vec2::ZERO;
                for row in range {
                    let (r, resp) = ui.allocate_exact_size(Vec2::new(width, self.row_h), Sense::click());
                    let resp: Response = resp;
                    let selected = self.selected == Some(row);
                    if selected {
                        ui.painter().rect_filled(r, CornerRadius::ZERO, p.selection_focused);
                    } else if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::ZERO, p.hover);
                    }
                    let mut x = r.min.x;
                    for (col, w) in widths.iter().enumerate() {
                        let cell_rect = Rect::from_min_size(
                            pos2(x + 6.0, r.min.y),
                            Vec2::new((w - 12.0).max(0.0), self.row_h),
                        );
                        let mut child = ui.new_child(
                            UiBuilder::new()
                                .id_salt(("gz-ui.table.cell", self.key, row, col))
                                .max_rect(cell_rect)
                                .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        );
                        child.set_clip_rect(cell_rect.intersect(ui.clip_rect()));
                        cell(&mut child, row, col);
                        x += w;
                    }
                    if resp.clicked() {
                        ev.clicked = Some(row);
                    }
                    if resp.double_clicked() {
                        ev.double_clicked = Some(row);
                    }
                    if resp.secondary_clicked() {
                        ev.context = Some(row);
                        ev.clicked = Some(row);
                    }
                }
            });
        ev
    }
}

/// A cell's text, cut at its end, in the text colour or muted.
pub fn cell_text(ui: &mut Ui, text_: &str, muted: bool) {
    let p = palette(ui.ctx());
    let color = if muted { p.text_secondary } else { p.text };
    text::label(ui, text_, Font::Body, color, Clip::End);
}

#[cfg(test)]
mod tests {
    use super::apply_move;

    #[test]
    fn a_row_moves_to_the_place_it_was_dropped_on() {
        let mut v = vec!['a', 'b', 'c', 'd'];
        apply_move(&mut v, 0, 2);
        assert_eq!(v, vec!['b', 'c', 'a', 'd']);
        apply_move(&mut v, 3, 0);
        assert_eq!(v, vec!['d', 'b', 'c', 'a']);
        apply_move(&mut v, 9, 0);
        assert_eq!(v.len(), 4);
    }
}
