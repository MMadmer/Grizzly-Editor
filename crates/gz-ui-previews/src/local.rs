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
