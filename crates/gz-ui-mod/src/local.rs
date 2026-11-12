//! Widgets the kit lacks, local to this crate until a second surface wants them
//! (docs/UI_DESIGN.md, section 4: "A widget the kit lacks is written as a local one").

use egui::{
    Align, CornerRadius, Id, Key, LayerId, Layout, Margin, Popup, Rect, Response, ScrollArea, Sense, Stroke,
    StrokeKind, TextEdit, TextureId, Ui, UiBuilder, Vec2, pos2,
};
use gz_ui::menu::menu_popup;
use gz_ui::units::em;
use gz_ui::{Font, Icon, MenuItem, SearchField, Tone, icons, menu_item, metrics, palette};

/// A one-line text field with a hint while it is empty, as wide as it gets (the kit's
/// `text_field` has no hint).
pub fn local_text_field(ui: &mut Ui, id_salt: &str, value: &mut String, hint: &str) -> Response {
    let id = ui.make_persistent_id(("gz-ui-mod.text", id_salt));
    let w = ui.available_width().max(em(3.0));
    gz_ui::form::hint_field(ui, id, value, hint, w)
}

/// A password field: the text is never shown.
pub fn local_password_field(
    ui: &mut Ui,
    id_salt: &str,
    value: &mut String,
    hint: &str,
    width: f32,
) -> Response {
    let id = ui.make_persistent_id(("gz-ui-mod.password", id_salt));
    let resp = ui.add(
        TextEdit::singleline(value)
            .id(id)
            .password(true)
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 4))
            .hint_text(hint)
            .desired_width(width)
            .min_size(Vec2::new(width, metrics::FRAME)),
    );
    gz_ui::audit::control(ui.ctx(), hint, true, None, None, resp.rect);
    resp
}

/// A switch with its label after it; a click on either flips it. `width` is the room it takes.
pub fn local_check(ui: &mut Ui, id_salt: &str, label: &str, value: &mut bool, width: f32) -> bool {
    let id = ui.make_persistent_id(("gz-ui-mod.check", id_salt));
    gz_ui::form::check(ui, id, label, value, width)
}

/// A block inside one property row: what `add` draws flows top-down from the value's left edge,
/// and the row grows to hold it (the row's own line is laid out left to right and centred).
pub fn local_value_column<R>(ui: &mut Ui, id_salt: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let origin = ui.max_rect().min;
    let w = ui.available_width().max(em(3.0));
    let rect = Rect::from_min_size(origin, Vec2::new(w, f32::INFINITY));
    let mut child = ui.new_child(
        UiBuilder::new()
            .id_salt(("gz-ui-mod.column", id_salt))
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    let r = add(&mut child);
    let used = child.min_rect();
    ui.allocate_rect(
        Rect::from_min_max(origin, pos2(origin.x + w, used.max.y.max(origin.y + metrics::FRAME))),
        Sense::hover(),
    );
    r
}

/// A picture in a frame of `side` points: the texture, or the frame and a picture glyph while
/// there is none, so the row does not jump.
pub fn local_picture(ui: &mut Ui, texture: Option<TextureId>, side: f32) -> Response {
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    if ui.is_rect_visible(rect) {
        let radius = CornerRadius::same(metrics::RADIUS);
        ui.painter().rect_filled(rect, radius, p.inset);
        match texture {
            Some(id) => {
                ui.painter().image(
                    id,
                    rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            None => {
                ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, p.border_subtle), StrokeKind::Inside);
                let glyph = 32.0_f32.min(side * 0.5);
                icons::draw(
                    ui.painter(),
                    Icon::Picture,
                    Rect::from_center_size(rect.center(), Vec2::splat(glyph)),
                    p.text_muted,
                );
            }
        }
    }
    resp
}

/// A row of a [`local_pick_list`]: what it picks, what it shows, and its tooltip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickRow {
    pub value: String,
    pub label: String,
    pub tip: String,
}

/// What a [`local_pick_list`] is.
pub struct PickList<'a> {
    pub popup: Id,
    /// Where it opens: under this rectangle.
    pub anchor: Rect,
    pub layer: LayerId,
    pub hint: &'a str,
    /// Said when no row is left.
    pub empty: &'a str,
    /// It just opened: the search takes the keyboard.
    pub focus: bool,
}

/// A searchable list in a popup (Add Module..., the Function picker): a search field, then the
/// rows `rows` makes of what is typed; a click or Enter (the first row) picks. Open it with
/// `Popup::open_id(ctx, popup)`. Returns the value picked.
pub fn local_pick_list(
    ctx: &egui::Context,
    spec: &PickList<'_>,
    search: &mut String,
    rows: impl FnOnce(&str) -> Vec<PickRow>,
) -> Option<String> {
    if !Popup::is_id_open(ctx, spec.popup) {
        return None;
    }
    let width = em(24.0);
    let p = palette(ctx);
    menu_popup(ctx, spec.popup, spec.anchor, spec.layer, |ui| {
        ui.set_width(width);
        let field =
            SearchField::new(search).hint(spec.hint).width(width).id_salt("gz-ui-mod.pick_search").show(ui);
        if spec.focus {
            field.request_focus();
        }
        let list = rows(search.as_str());
        let mut picked = None;
        // Enter takes the first row
        if !list.is_empty() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Enter)) {
            picked = list.first().map(|r| r.value.clone());
        }
        ui.add_space(4.0);
        ScrollArea::vertical()
            .id_salt("gz-ui-mod.pick_rows")
            .max_height(240.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if list.is_empty() {
                    let color = p.tone(Tone::Muted);
                    ui.add(
                        egui::Label::new(egui::RichText::new(spec.empty).font(Font::Body.id()).color(color))
                            .wrap(),
                    );
                }
                for row in &list {
                    let out =
                        ui.push_id(row.value.as_str(), |ui| menu_item(ui, &row.label, MenuItem::default()));
                    if !row.tip.is_empty() {
                        out.response.on_hover_text(row.tip.as_str());
                    }
                    if out.inner {
                        picked = Some(row.value.clone());
                    }
                }
            });
        if picked.is_some() {
            ui.close();
        }
        picked
    })
    .flatten()
}
