//! Widgets the kit lacks, local to this crate until a second surface wants them
//! (docs/UI_DESIGN.md, section 4).

use egui::{Align2, CornerRadius, Painter, Pos2, Rect, Sense, Ui, Vec2, pos2};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Palette, palette};

/// The checkerboard a picture with alpha is judged against: only the cells inside `clip` are
/// painted, lined up with `origin` so they move with the picture (a zoomed-in 4096 texture is
/// millions of cells, a canvas a few hundred).
pub fn local_checker(painter: &Painter, p: &Palette, origin: Pos2, clip: Rect) {
    let cell = 8.0;
    if clip.width() <= 0.0 || clip.height() <= 0.0 {
        return;
    }
    painter.rect_filled(clip, CornerRadius::ZERO, p.input);
    let col0 = ((clip.min.x - origin.x) / cell).floor() as i64;
    let row0 = ((clip.min.y - origin.y) / cell).floor() as i64;
    let mut row = row0;
    while origin.y + row as f32 * cell < clip.max.y {
        let mut col = col0 + ((col0 + row) & 1);
        while origin.x + col as f32 * cell < clip.max.x {
            let a = pos2(origin.x + col as f32 * cell, origin.y + row as f32 * cell);
            let r = Rect::from_min_max(a.max(clip.min), (a + Vec2::splat(cell)).min(clip.max));
            if r.width() > 0.0 && r.height() > 0.0 {
                painter.rect_filled(r, CornerRadius::ZERO, p.hover);
            }
            col += 2;
        }
        row += 1;
    }
}

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

/// A folding header outside a property grid (the C++ `XUI::Section`): a chevron and the title,
/// the whole row a click target; `open` is its state until it is clicked. Returns whether it is
/// open.
pub fn local_section(ui: &mut Ui, id_salt: &str, title: &str, open: bool) -> bool {
    let id = ui.make_persistent_id(("gz-ui-previews.section", id_salt));
    gz_ui::form::raised_section(ui, id, title, open, None)
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
