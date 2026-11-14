//! Widgets the kit lacks, local to this crate until a second surface wants them
//! (docs/UI_DESIGN.md, section 4).

use egui::{Align2, CornerRadius, Painter, Rect, Sense, Ui, Vec2};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, palette};

/// A preview's title (Title type, ellipsized) and its facts under it (Small, muted).
pub fn local_heading(ui: &mut Ui, title: &str, title_clip: Clip, facts: &[(&str, Clip)]) {
    let p = palette(ui.ctx());
    text::label(ui, title, Font::Title, p.text, title_clip);
    for (line, clip) in facts {
        if !line.is_empty() {
            text::label(ui, line, Font::Small, p.text_muted, *clip);
        }
    }
}

/// A muted line of Small text at the left and another at the right of one row: a timeline's
/// first and last frame, a curve's two ends.
pub fn local_ends(ui: &mut Ui, left: &str, right: &str) {
    let p = palette(ui.ctx());
    let h = Font::Small.line();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
    if ui.is_rect_visible(rect) {
        let font = Font::Small.id();
        let half = Some((rect.width() * 0.5 - 4.0).max(0.0));
        text::paint(
            ui.painter(),
            &font,
            rect.left_center(),
            Align2::LEFT_CENTER,
            p.text_muted,
            left,
            half,
            Clip::End,
        );
        text::paint(
            ui.painter(),
            &font,
            rect.right_center(),
            Align2::RIGHT_CENTER,
            p.text_muted,
            right,
            half,
            Clip::End,
        );
    }
}

/// A 2 pt line of the value colour across a rect: the swatch's and the timeline's frame.
pub fn local_frame(painter: &Painter, rect: Rect, color: egui::Color32, radius: u8) {
    painter.rect_stroke(
        rect,
        CornerRadius::same(radius),
        egui::Stroke::new(1.0, color),
        egui::StrokeKind::Inside,
    );
}

/// A texture of the library picked from a searchable list, or None: the chooser of a
/// description's detail, bump and normal map (`CreateChoose(smTexture)`). Returns the name
/// picked ("" for None).
pub fn local_texture_pick(ui: &mut Ui, id_salt: &str, value: &str, names: &[&str]) -> Option<String> {
    let ctx = ui.ctx().clone();
    let search_id = ui.make_persistent_id(("gz-ui-previews.pick", id_salt));
    let mut picked = None;
    let w = ui.available_width();
    let shown = if value.is_empty() { "None" } else { gz_ui::grid::leaf_name(value) };
    let resp = egui::ComboBox::from_id_salt(("gz-ui-previews.pick.combo", id_salt))
        .selected_text(shown)
        .width(w)
        .height(320.0)
        .show_ui(ui, |ui| {
            let mut search = ctx.data(|d| d.get_temp::<String>(search_id)).unwrap_or_default();
            gz_ui::SearchField::new(&mut search).id_salt(id_salt).width(gz_ui::units::em(16.0)).show(ui);
            if gz_ui::menu_item(
                ui,
                "None",
                gz_ui::MenuItem { selected: value.is_empty(), ..gz_ui::MenuItem::default() },
            ) {
                picked = Some(String::new());
            }
            // a few hundred rows at most: the search narrows the rest
            for name in names.iter().filter(|n| gz_ui::search::matches(&search, n)).take(300) {
                if gz_ui::menu_item(
                    ui,
                    name,
                    gz_ui::MenuItem { selected: *name == value, ..gz_ui::MenuItem::default() },
                ) {
                    picked = Some((*name).to_owned());
                }
            }
            ctx.data_mut(|d| d.insert_temp(search_id, search));
        });
    resp.response.on_hover_text(if value.is_empty() { "None" } else { value });
    picked
}

/// The wheel's turn this frame in notches (a line is one, 50 points one, a page ten): what a
/// canvas zooms by. Positive turns away from the author.
#[must_use]
pub fn local_wheel(ui: &Ui) -> f32 {
    ui.input(|i| {
        i.raw
            .events
            .iter()
            .map(|e| match e {
                egui::Event::MouseWheel { unit, delta, .. } => match unit {
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Point => delta.y / 50.0,
                    egui::MouseWheelUnit::Page => delta.y * 10.0,
                },
                _ => 0.0,
            })
            .sum()
    })
}
