//! The list of a library tab: a search, then one row per item - its
//! leaf, then its folder, muted, giving way first - in the order of the leaves, by folder only
//! among equal names. Only the rows in view are laid out.

use std::cmp::Ordering;

use egui::{Align2, Key, Rect, Sense, Ui, Vec2, pos2};
use gz_ui::list::list;
use gz_ui::search::matches;
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{Font, Icon, SearchField, Tx, metrics, palette, state};

use crate::paths::leaf;

fn ci(a: &str, b: &str) -> Ordering {
    a.chars().flat_map(char::to_lowercase).cmp(b.chars().flat_map(char::to_lowercase))
}

/// `Before`: by the leaf, then by the whole key, ignoring case. Sorted by the whole key,
/// "cop\test_la_color" sat among the c's and the list read as unsorted.
#[must_use]
pub fn before(a: &str, b: &str) -> Ordering {
    ci(leaf(a), leaf(b)).then_with(|| ci(a, b))
}

/// The rows a list shows: the visible keys the search matches, in the list's order.
#[must_use]
pub fn shown(keys: &[&str], visible: &dyn Fn(usize) -> bool, search: &str) -> Vec<usize> {
    let mut out: Vec<usize> =
        (0..keys.len()).filter(|&i| visible(i) && keys.get(i).is_some_and(|k| matches(search, k))).collect();
    out.sort_by(|&a, &b| before(keys.get(a).copied().unwrap_or(""), keys.get(b).copied().unwrap_or("")));
    out
}

/// `Find`: the key named exactly (any case), else the first in the list's order that the name
/// matches as a search.
#[must_use]
pub fn find(keys: &[&str], name: &str) -> Option<usize> {
    if name.is_empty() {
        return None;
    }
    if let Some(i) = keys.iter().position(|k| k.eq_ignore_ascii_case(name) || ci(k, name) == Ordering::Equal)
    {
        return Some(i);
    }
    let mut first: Option<usize> = None;
    for (i, k) in keys.iter().enumerate() {
        if matches(name, k)
            && first.is_none_or(|f| before(k, keys.get(f).copied().unwrap_or("")) == Ordering::Less)
        {
            first = Some(i);
        }
    }
    first
}

/// What an empty list says.
pub struct Empty<'a> {
    pub title: Tx<'a>,
    pub text: Tx<'a>,
    pub action: Option<Tx<'a>>,
}

/// What a frame of the list did.
#[derive(Default)]
pub struct ListOutput {
    /// The row picked: a click or an arrow key.
    pub picked: Option<usize>,
    /// The empty state's action was pressed.
    pub acted: bool,
}

/// The list's view state: the search, and the selection it last brought into view.
#[derive(Default)]
pub struct LibraryList {
    pub search: String,
    last_selected: String,
    focused: bool,
}

impl LibraryList {
    /// Draws the list into `ui` (its whole room). `rows` are what [`shown`] answers for the
    /// current search; `selected` is the key on show.
    pub fn draw(
        &mut self,
        ui: &mut Ui,
        id: &str,
        keys: &[&str],
        rows: &[usize],
        selected: &str,
        empty: &Empty<'_>,
    ) -> ListOutput {
        let mut out = ListOutput::default();
        let w = ui.available_width();
        SearchField::new(&mut self.search).id_salt(id).width(w).show(ui);
        ui.add_space(4.0);
        // the selected row among the shown ones; a selection made elsewhere (MCP, a reload) is
        // brought into view
        let at = if selected.is_empty() {
            None
        } else {
            rows.iter().position(|&i| keys.get(i).is_some_and(|k| *k == selected))
        };
        let scroll_to = if self.last_selected != selected {
            self.last_selected = selected.to_owned();
            at
        } else {
            None
        };
        if self.focused && !rows.is_empty() && !ui.ctx().egui_wants_keyboard_input() {
            let step = ui.input(|i| {
                if i.key_pressed(Key::ArrowDown) {
                    1i64
                } else if i.key_pressed(Key::ArrowUp) {
                    -1
                } else {
                    0
                }
            });
            if step != 0 {
                let next = at.map_or(0, |a| (a as i64 + step).clamp(0, rows.len() as i64 - 1) as usize);
                if Some(next) != at {
                    out.picked = rows.get(next).copied();
                }
            }
        }
        let p = palette(ui.ctx());
        let search_on = !self.search.is_empty();
        let mut clicked = false;
        list(ui, id, Vec2::ZERO, |l| {
            if rows.is_empty() {
                let ui = l.ui();
                let (icon, title, text_, action) = if search_on {
                    (
                        Icon::Search,
                        Tx::from("No matches"),
                        Tx::from("Nothing in the library matches the search."),
                        None,
                    )
                } else {
                    (
                        Icon::Folder,
                        empty.title.borrow(),
                        empty.text.borrow(),
                        empty.action.as_ref().map(Tx::borrow),
                    )
                };
                out.acted = state::empty_state(ui, icon, title, text_, action);
                return;
            }
            let row_h = metrics::ROW;
            let top = l.ui().cursor().min.y;
            if let Some(r) = scroll_to {
                let rect = Rect::from_min_size(
                    pos2(l.ui().cursor().min.x, top + r as f32 * row_h),
                    Vec2::new(1.0, row_h),
                );
                l.ui().scroll_to_rect(rect, None);
            }
            let clip = l.ui().clip_rect();
            let first = (((clip.min.y - top) / row_h).floor().max(0.0) as usize).min(rows.len());
            let last = ((((clip.max.y - top) / row_h).ceil().max(0.0)) as usize + 1).min(rows.len());
            if first > 0 {
                l.ui().add_space(first as f32 * row_h);
            }
            for (r, &i) in rows.iter().enumerate().take(last).skip(first) {
                let key = keys.get(i).copied().unwrap_or("");
                let resp = l.row(key, Some(r) == at, |ui| {
                    // the leaf names the item; its folder follows, muted, and gives way first
                    let room = ui.available_width() - 6.0;
                    let name = leaf(key);
                    let font = Font::Body.id();
                    let rect = ui.max_rect();
                    let y = rect.center().y;
                    let shown_w = text::width(ui.ctx(), &font, Tx::raw(name)).min(room);
                    text::paint(
                        ui.painter(),
                        &font,
                        pos2(rect.min.x, y),
                        Align2::LEFT_CENTER,
                        p.text,
                        Tx::raw(name),
                        Some(room.max(0.0)),
                        Clip::End,
                    );
                    let rest = room - shown_w - 8.0;
                    if key.len() > name.len() && rest > em(3.0) {
                        let folder = &key[..key.len() - name.len() - 1];
                        text::paint(
                            ui.painter(),
                            &font,
                            pos2(rect.min.x + shown_w + 8.0, y),
                            Align2::LEFT_CENTER,
                            p.text_muted,
                            Tx::raw(folder),
                            Some(rest),
                            Clip::Start,
                        );
                    }
                    ui.allocate_rect(rect, Sense::hover());
                });
                if resp.clicked() {
                    out.picked = Some(i);
                    clicked = true;
                }
            }
            if last < rows.len() {
                l.ui().add_space((rows.len() - last) as f32 * row_h);
            }
        });
        if clicked {
            self.focused = true;
        } else if ui.input(|i| i.pointer.any_click()) && !ui.ui_contains_pointer() {
            self.focused = false;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_is_in_the_order_of_the_leaves() {
        let keys = ["cop\\test_la_color", "a\\zz", "b\\alpha", "c\\Alpha"];
        let rows = shown(&keys, &|_| true, "");
        let names: Vec<&str> = rows.iter().map(|&i| keys[i]).collect();
        assert_eq!(names, vec!["b\\alpha", "c\\Alpha", "cop\\test_la_color", "a\\zz"]);
        let rows = shown(&keys, &|i| i != 1, "alpha");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn find_takes_the_name_then_the_first_match() {
        let keys = ["x\\fire", "a\\campfire", "b\\fire_big"];
        assert_eq!(find(&keys, "X\\FIRE"), Some(0));
        assert_eq!(find(&keys, "big"), Some(2));
        // "fire" matches all three as a search; the named one wins
        assert_eq!(find(&keys, "campfire"), Some(1));
        assert_eq!(find(&keys, "nothing"), None);
        assert_eq!(find(&keys, ""), None);
    }
}
