//! The Level tab's part of the status bar: the centre - the selection count, the grid and the
//! angle - and the "Build: Game Files · AI Map" chip of the steps the scene's changes left stale.
//! The chip opens the same popover as Build Level's menu: every step with its state, building
//! one when picked, Build Level and Rebuild All.
//!
//! What is stale is the level build's status (`gz-levelbuild`'s [`LevelBuild::status_now`]): a
//! query keyed on the scene's revision and the build's record, never polled, right after a
//! restart because the record lives beside the level's build.

use std::sync::Arc;

use egui::{Align2, CornerRadius, Id, Sense, Ui, Vec2, pos2};
use gz_app::{DocCx, EditorSettings, ShellBuilder, menu_row};
use gz_editor::Editor;
use gz_levelbuild::{LevelBuild, StepState};
use gz_scene::SceneService;
use gz_ui::text::{self, Clip};
use gz_ui::{Font, TipExt, Tone, Tx, audit, palette, tr};

use crate::actions as a;

/// The titles of the steps a build would run now.
#[must_use]
pub fn pending(editor: &Arc<Editor>) -> Vec<&'static str> {
    let Some(st) = LevelBuild::status_now(editor) else { return Vec::new() };
    st.steps
        .iter()
        .filter(|s| matches!(s.state, StepState::Stale | StepState::Never | StepState::Failed))
        .map(|s| s.title)
        .collect()
}

/// Whether the chip is up: the scene has objects and a step to build.
#[must_use]
pub fn rebuild_pending(editor: &Arc<Editor>) -> bool {
    let objects = editor
        .project_opt()
        .and_then(|s| s.services.get::<SceneService>())
        .is_some_and(|svc| !svc.store.snapshot().objects.is_empty());
    objects && !pending(editor).is_empty()
}

/// The chip's words: "Build: Game Files · AI Map".
#[must_use]
pub fn chip_label(titles: &[&str]) -> String {
    format!("Build: {}", titles.join(" \u{b7} "))
}

/// The centre of the status bar on Level.
pub fn centre(ui: &mut Ui, cx: &DocCx<'_>) {
    let p = palette(ui.ctx());
    let selected = cx
        .editor
        .project_opt()
        .and_then(|s| s.services.get::<SceneService>())
        .map_or(0, |svc| svc.store.snapshot().selection.len());
    let settings = EditorSettings::of(cx.editor);
    let grid = settings.pref("snap_grid_step").unwrap_or(0.1);
    let angle = settings.pref("snap_angle_step").unwrap_or(5.625);
    let text_ = tr!(
        "{n} selected  \u{b7}  Grid {grid} m  \u{b7}  Angle {angle}\u{b0}",
        n = selected,
        grid = fmt_g(grid),
        angle = fmt_g(angle)
    );
    text::label(ui, text_, Font::Small, p.text_secondary, Clip::End);
}

/// A number with as many decimals as it needs.
fn fmt_g(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".to_owned() } else { s.to_owned() }
}

/// The popover's rows: the steps with their state, Build Level and Rebuild All.
pub fn popover(ui: &mut Ui, cx: &mut DocCx<'_>) {
    crate::menus::step_rows(ui, cx);
    ui.separator();
    menu_row(ui, cx, a::BUILD_LEVEL, None);
    menu_row(ui, cx, a::REBUILD_ALL, None);
}

fn draw_chip(ui: &mut Ui, cx: &mut DocCx<'_>) {
    let editor = cx.editor.clone();
    if !rebuild_pending(&editor) {
        return;
    }
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let titles = pending(&editor);
    // the English names the chip to the audit; the window reads it in its language
    let label = chip_label(&titles);
    let steps: Vec<String> = titles.iter().copied().map(|t| crate::menus::step_tx(t).text(&ctx)).collect();
    let shown = tr!("Build: {steps}", steps = steps.join(" \u{b7} ")).text(&ctx);
    let font = Font::Small.id();
    let w = text::width(&ctx, &font, Tx::raw(&shown)) + 12.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 20.0), Sense::hover());
    let id = Id::unique("gz-ui-level.rebuild_chip");
    let resp = ui.interact(rect, id, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), p.hover);
    }
    ui.painter().text(
        pos2(rect.min.x + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        shown,
        font,
        p.tone(Tone::Warning),
    );
    audit::control(&ctx, &label, true, None, None, rect);
    let resp = resp.tip("The scene changed since these steps were built. Build Level builds them in order.");
    let popup = id.with("popup");
    if resp.clicked() {
        egui::Popup::toggle_id(&ctx, popup);
    }
    if gz_ui::menu::take_popup_request(&ctx, "rebuild") {
        egui::Popup::open_id(&ctx, popup);
    }
    let layer = ui.layer_id();
    gz_ui::menu::menu_popup(&ctx, popup, rect, layer, |ui| popover(ui, cx));
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.status_chip(0, draw_chip);
}

#[cfg(test)]
mod tests {
    use super::{chip_label, fmt_g};

    #[test]
    fn numbers_print_with_the_decimals_they_need() {
        assert_eq!(fmt_g(0.1), "0.1");
        assert_eq!(fmt_g(5.625), "5.625");
        assert_eq!(fmt_g(1.0), "1");
    }

    #[test]
    fn the_chip_names_the_stale_steps() {
        assert_eq!(chip_label(&["Game Files", "AI Map"]), "Build: Game Files \u{b7} AI Map");
    }
}
