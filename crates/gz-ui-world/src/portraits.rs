//! Portraits: a portrait is a rectangle of a sheet that holds a hundred of them. Every sheet is
//! read and decoded on the pools the first time a face on it is shown, uploaded once and kept
//! while the project is open.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{
    ColorImage, CornerRadius, Id, Popup, Rect, Response, ScrollArea, Sense, Stroke, StrokeKind,
    TextureHandle, TextureId, TextureOptions, Ui, Vec2, pos2,
};
use gz_editor::Editor;
use gz_game::GameLink;
use gz_nq::PickerEntry;
use gz_ui::menu::menu_popup;
use gz_ui::text;
use gz_ui::units::em;
use gz_ui::{Icon, MenuItem, SearchField, Tone, icons, menu_item, palette};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::call::post_ui;
use crate::lists::{Indexed, ListsView};
use crate::picker::paint_field;

/// The largest sheet read.
const SHEET_READ_LIMIT: u64 = 128 << 20;

enum Slot {
    Loading,
    Failed,
    Ready { texture: TextureHandle, size: [u32; 2] },
}

/// The portrait sheets of the open project (`session.services`).
#[derive(Default)]
pub struct Sheets {
    slots: Mutex<FxHashMap<String, Slot>>,
}

/// Where a portrait is: its sheet (below `textures\`), whether the project holds it, and its
/// rectangle in the sheet's pixels. `extra` is `[project:]<sheet>|x|y|w|h`.
fn place(extra: &str) -> Option<(bool, &str, [f32; 4])> {
    let (project, rest) = match extra.strip_prefix("project:") {
        Some(r) => (true, r),
        None => (false, extra),
    };
    let mut parts = rest.split('|');
    let sheet = parts.next().filter(|s| !s.is_empty())?;
    let mut n = [0.0_f32; 4];
    for v in &mut n {
        *v = parts.next()?.trim().parse::<f32>().ok()?;
    }
    if parts.next().is_some() {
        return None;
    }
    Some((project, sheet, n))
}

/// A sheet's pixels as egui takes them; `None` when it does not read. Blocking.
fn read_sheet(bytes: &[u8]) -> Option<ColorImage> {
    let loaded = gz_fmt_tex::load_image(bytes, Some("dds")).ok()?;
    let img = loaded.image;
    Some(ColorImage::from_rgba_unmultiplied([img.width as usize, img.height as usize], &img.rgba))
}

impl Sheets {
    /// The sheets of the open project; `None` without one.
    #[must_use]
    pub fn of(editor: &Editor) -> Option<Arc<Self>> {
        let session = editor.project_opt()?;
        Some(session.services.get_or_init(Self::default))
    }

    /// The texture and the rectangle of a portrait, once its sheet is up; the first ask starts
    /// the read.
    pub fn portrait(self: &Arc<Self>, editor: &Arc<Editor>, e: &PickerEntry) -> Option<(TextureId, Rect)> {
        let (project, sheet, [x, y, w, h]) = place(&e.extra)?;
        let key = format!("{}|{}", if project { "p" } else { "g" }, sheet);
        let mut slots = self.slots.lock();
        match slots.get(&key) {
            Some(Slot::Ready { texture, size }) => {
                let (sw, sh) = (size[0].max(1) as f32, size[1].max(1) as f32);
                Some((
                    texture.id(),
                    Rect::from_min_max(pos2(x / sw, y / sh), pos2((x + w) / sw, (y + h) / sh)),
                ))
            }
            Some(_) => None,
            None => {
                slots.insert(key.clone(), Slot::Loading);
                drop(slots);
                self.load(editor, key, project, sheet.to_owned());
                None
            }
        }
    }

    fn load(self: &Arc<Self>, editor: &Arc<Editor>, key: String, project: bool, sheet: String) {
        let (ed, me) = (editor.clone(), self.clone());
        let _task = editor.exec.spawn(async move {
            let image = if project {
                let root = ed.project_opt().map(|s| s.root.clone());
                ed.exec
                    .io(move || {
                        let path: PathBuf = root?
                            .join("gamedata")
                            .join("textures")
                            .join(format!("{}.dds", sheet.replace('/', "\\")));
                        if std::fs::metadata(&path).ok()?.len() > SHEET_READ_LIMIT {
                            return None;
                        }
                        read_sheet(&std::fs::read(&path).ok()?)
                    })
                    .await
            } else {
                match GameLink::of(&ed) {
                    Ok(link) => match link.files().await {
                        Ok(files) => {
                            let rel = format!("textures\\{sheet}.dds").to_lowercase();
                            ed.exec.io(move || files.read(&rel).ok().and_then(|b| read_sheet(&b))).await
                        }
                        Err(_) => None,
                    },
                    Err(_) => None,
                }
            };
            post_ui(&ed, move |ctx| {
                let slot = match image {
                    Some(img) => {
                        let size = [img.size[0] as u32, img.size[1] as u32];
                        Slot::Ready {
                            texture: ctx.load_texture(
                                format!("gz-ui-world.sheet.{key}"),
                                img,
                                TextureOptions::LINEAR,
                            ),
                            size,
                        }
                    }
                    None => Slot::Failed,
                };
                me.slots.lock().insert(key, slot);
            });
        });
    }
}

/// The face the player sees, `height` tall (and 1.42 times as wide), or a frame with a person in
/// it while there is none.
pub fn draw_portrait(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    sheets: Option<&Arc<Sheets>>,
    lists: &ListsView,
    icon: &str,
    height: f32,
) -> Response {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(height * 1.42, height), Sense::hover());
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(4);
        ui.painter().rect_filled(rect, radius, p.inset);
        let face = lists.icons.get(icon).zip(sheets).and_then(|(e, s)| s.portrait(editor, e));
        match face {
            Some((texture, uv)) => {
                ui.painter().image(texture, rect, uv, egui::Color32::WHITE);
            }
            None => {
                ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
                let glyph = height.min(40.0) * 0.6;
                icons::draw(
                    ui.painter(),
                    Icon::Person,
                    Rect::from_center_size(rect.center(), Vec2::splat(glyph)),
                    p.text_muted,
                );
            }
        }
    }
    resp.on_hover_text(if icon.is_empty() { "No portrait" } else { icon })
}

/// The portraits a search shows, worked out once per query and per read of the list.
#[derive(Clone, Default)]
struct Shown {
    query: String,
    serial: u64,
    rows: Arc<Vec<u32>>,
}

/// The portrait, picked from a gallery of every face the linked game and the project have: by
/// looking, not by remembering `ui_inGame2_neutral_2_mask`. Only the rows in view
/// are laid out, and only their sheets read. Returns whether the value changed.
pub fn local_portrait_gallery(
    ui: &mut Ui,
    salt: &str,
    value: &mut String,
    editor: &Arc<Editor>,
    sheets: Option<&Arc<Sheets>>,
    lists: &ListsView,
) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let list: &Indexed = &lists.icons;
    let id = Id::unique(("gz-ui-world.gallery", salt));
    let unknown = !value.is_empty() && list.get(value).is_none() && !list.is_empty();
    let shown_text = if value.is_empty() { "None" } else { value.as_str() };
    let color = if unknown {
        p.warning
    } else if value.is_empty() {
        p.text_secondary
    } else {
        p.text
    };
    let (resp, _) = paint_field(ui, shown_text, color, !lists.loaded);
    gz_ui::audit::control(
        &ctx,
        shown_text,
        lists.loaded,
        (!lists.loaded).then_some("The linked game's lists are being read"),
        None,
        resp.rect,
    );
    let resp = if unknown {
        resp.on_hover_text(format!("'{value}' is not in the linked game or the project: kept as written"))
    } else {
        resp
    };
    // a click toggles it; `characters.view show=icon` puts it up to be looked at
    let asked = lists.loaded && gz_ui::menu::take_popup_request(&ctx, salt);
    if (resp.clicked() && !Popup::is_id_open(&ctx, id)) || asked {
        ctx.data_mut(|d| {
            d.insert_temp(id.with("search"), String::new());
            d.insert_temp(id.with("focus"), true);
        });
        Popup::open_id(&ctx, id);
    } else if resp.clicked() {
        Popup::close_id(&ctx, id);
    }
    if !Popup::is_id_open(&ctx, id) {
        return false;
    }
    let width = resp.rect.width().max(em(30.0));
    let cell = Vec2::new(em(6.2), em(4.4));
    let gap = 4.0;
    let mut picked: Option<String> = None;
    let _ = menu_popup(&ctx, id, resp.rect, ui.layer_id(), |ui| {
        ui.set_width(width);
        let mut query = ctx.data(|d| d.get_temp::<String>(id.with("search"))).unwrap_or_default();
        let field = SearchField::new(&mut query)
            .hint("Search portraits")
            .width(width)
            .id_salt("gz-ui-world.gallery_search")
            .show(ui);
        if ctx.data_mut(|d| d.remove_temp::<bool>(id.with("focus"))).is_some() {
            field.request_focus();
        }
        ctx.data_mut(|d| d.insert_temp(id.with("search"), query.clone()));
        ui.add_space(4.0);
        if menu_item(ui, "None", MenuItem { selected: value.is_empty(), ..MenuItem::default() }) {
            picked = Some(String::new());
        }
        let key = id.with("shown");
        let rows = match ctx.data(|d| d.get_temp::<Shown>(key)) {
            Some(s) if s.query == query && s.serial == list.serial => s.rows,
            _ => {
                let rows: Vec<u32> = list
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| {
                        query.trim().is_empty()
                            || gz_ui::search::matches(&query, &format!("{} {}", e.id, e.name))
                    })
                    .filter_map(|(i, _)| u32::try_from(i).ok())
                    .collect();
                let rows = Arc::new(rows);
                ctx.data_mut(|d| {
                    d.insert_temp(
                        key,
                        Shown { query: query.clone(), serial: list.serial, rows: rows.clone() },
                    )
                });
                rows
            }
        };
        if rows.is_empty() {
            let _ = text::status(ui, Tone::Muted, "Nothing matches.");
            return;
        }
        let cols = (((width + gap) / (cell.x + gap)).floor() as usize).max(1);
        let lines = rows.len().div_ceil(cols);
        ui.spacing_mut().item_spacing.y = gap;
        ScrollArea::vertical()
            .id_salt(id.with("grid"))
            .max_height(360.0)
            .auto_shrink([false, true])
            .show_rows(ui, cell.y, lines, |ui, range| {
                for line in range {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for c in 0..cols {
                            let Some(e) =
                                rows.get(line * cols + c).and_then(|r| list.entries.get(*r as usize))
                            else {
                                break;
                            };
                            let (r, cell_resp) = ui.allocate_exact_size(cell, Sense::click());
                            if ui.is_rect_visible(r) {
                                let radius = CornerRadius::same(3);
                                ui.painter().rect_filled(
                                    r,
                                    radius,
                                    if cell_resp.hovered() { p.hover } else { p.inset },
                                );
                                match sheets.and_then(|s| s.portrait(editor, e)) {
                                    Some((texture, uv)) => {
                                        ui.painter().image(texture, r.shrink(2.0), uv, egui::Color32::WHITE);
                                    }
                                    // its sheet is still on its way (or does not read): the frame
                                    // the player would see, never the id in place of a face
                                    None => {
                                        let glyph = (r.height() * 0.4).min(28.0);
                                        icons::draw(
                                            ui.painter(),
                                            Icon::Person,
                                            Rect::from_center_size(r.center(), Vec2::splat(glyph)),
                                            p.text_muted,
                                        );
                                    }
                                }
                                if e.id == *value {
                                    ui.painter().rect_stroke(
                                        r,
                                        radius,
                                        Stroke::new(2.0, p.accent_bright),
                                        StrokeKind::Inside,
                                    );
                                }
                            }
                            let tip = if e.extra.starts_with("project:") {
                                format!("{}\nThe project's", e.id)
                            } else {
                                e.id.clone()
                            };
                            if cell_resp.on_hover_text(tip).clicked() {
                                picked = Some(e.id.clone());
                            }
                        }
                    });
                }
            });
    });
    match picked {
        Some(v) => {
            Popup::close_id(&ctx, id);
            let changed = v != *value;
            *value = v;
            changed
        }
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::place;

    #[test]
    fn a_portrait_names_its_sheet_and_rectangle() {
        assert_eq!(
            place("ui\\ui_actor_portrets|10|20|30|40"),
            Some((false, "ui\\ui_actor_portrets", [10.0, 20.0, 30.0, 40.0]))
        );
        assert_eq!(place("project:ui\\mine|0|0|165|108").map(|p| p.0), Some(true));
        assert_eq!(place("ui\\x|1|2|3"), None);
        assert_eq!(place("|1|2|3|4"), None);
        assert_eq!(place("ui\\x|1|2|3|4|5"), None);
    }
}
