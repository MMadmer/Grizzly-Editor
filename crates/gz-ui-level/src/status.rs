//! The Level tab's part of the status bar: the centre - the selection count, the grid and the
//! angle - and the "Build: Game Files · AI Map" chip of the steps the scene's changes left stale,
//! with a [Build] per step.
//!
//! What is stale is the level build's status (`gz-levelbuild`'s [`LevelBuild::status_now`]): a
//! query keyed on the scene's revision and the build's record, never polled, right after a
//! restart because the record lives beside the level's build.

use std::sync::Arc;

use egui::{Align2, CornerRadius, Id, Sense, Ui, Vec2, pos2};
use gz_app::{DocCx, EditorSettings, ShellBuilder};
use gz_editor::{Args, Editor, Value};
use gz_levelbuild::{LevelBuild, StepState};
use gz_scene::SceneService;
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Tone, audit, palette};

use crate::toolbar;

/// The steps a build would run now, with why each is not fresh.
fn pending(editor: &Arc<Editor>) -> Vec<(gz_levelbuild::Step, &'static str, String)> {
    let Some(st) = LevelBuild::status_now(editor) else { return Vec::new() };
    st.steps
        .iter()
        .filter(|s| matches!(s.state, StepState::Stale | StepState::Never | StepState::Failed))
        .map(|s| (s.step, s.title, s.why.clone()))
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
    let text_ =
        format!("{selected} selected  \u{b7}  Grid {}  \u{b7}  Angle {}\u{b0}", fmt_g(grid), fmt_g(angle));
    text::label(ui, &text_, Font::Small, p.text_secondary, Clip::End);
}

/// `%g`.
fn fmt_g(v: f64) -> String {
    let s = format!("{v:.6}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".to_owned() } else { s.to_owned() }
}

fn draw_chip(ui: &mut Ui, cx: &mut DocCx<'_>) {
    let editor = cx.editor.clone();
    if !rebuild_pending(&editor) {
        return;
    }
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let items = pending(&editor);
    let mut label = String::from("Build: ");
    label.push_str(&items.iter().map(|(_, t, _)| *t).collect::<Vec<_>>().join(" \u{b7} "));
    let font = Font::Small.id();
    let w = text::width(&ctx, &font, &label) + 12.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 20.0), Sense::hover());
    let id = Id::unique("gz-ui-level.rebuild_chip");
    let resp = ui.interact(rect, id, Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), p.hover);
    }
    ui.painter().text(
        pos2(rect.min.x + 6.0, rect.center().y),
        Align2::LEFT_CENTER,
        &label,
        font,
        p.tone(Tone::Warning),
    );
    audit::control(&ctx, &label, true, None, None, rect);
    let resp = resp
        .on_hover_text("The scene changed since these steps were built. Build Level builds them in order.");
    let popup = id.with("popup");
    if resp.clicked() {
        egui::Popup::toggle_id(&ctx, popup);
    }
    if gz_ui::menu::take_popup_request(&ctx, "rebuild") {
        egui::Popup::open_id(&ctx, popup);
    }
    let busy = toolbar::running(&editor);
    egui::Popup::new(popup, ctx.clone(), egui::PopupAnchor::ParentRect(rect), ui.layer_id())
        .kind(egui::PopupKind::Popup)
        .open_memory(None)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .align(egui::RectAlign::TOP_START)
        .gap(4.0)
        .frame(gz_ui::menu::popup_frame(&ctx))
        .show(|ui| {
            let name_w =
                items.iter().map(|(_, n, _)| text::width(&ctx, &Font::Body.id(), n)).fold(0.0, f32::max);
            let why = "A build, the compiler or the game is running.";
            for (step, name, reason) in &items {
                ui.horizontal(|ui| {
                    ui.allocate_ui(Vec2::new(name_w + 24.0, gz_ui::metrics::FRAME), |ui| {
                        text::label(ui, name, Font::Body, p.text, Clip::End).on_hover_text(reason)
                    });
                    if gz_ui::Button::new("Build").enabled(!busy, why).show(ui).clicked() {
                        let mut args = Args::new();
                        args.insert("only".into(), Value::String(step.name().to_owned()));
                        editor.post("level.build", args);
                        egui::Popup::close_id(&ctx, popup);
                    }
                });
            }
            ui.separator();
            if gz_ui::Button::new("Build Level").enabled(!busy, why).show(ui).clicked() {
                editor.post("level.build", Args::new());
                egui::Popup::close_id(&ctx, popup);
            }
        });
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.status_chip(0, draw_chip);
}

#[cfg(test)]
mod tests {
    use super::fmt_g;

    #[test]
    fn numbers_print_as_printf_g() {
        assert_eq!(fmt_g(0.1), "0.1");
        assert_eq!(fmt_g(5.625), "5.625");
        assert_eq!(fmt_g(1.0), "1");
    }
}
