//! The Level tab's part of the status bar (`UIMainForm::DrawLevelStatus`, `DrawStatusRight`):
//! the centre - the selection count, the grid and the angle - and the "Rebuild: Collision ·
//! AI Map · Game Graph" chip with a [Build] per item.
//!
//! What needs rebuilding is the level builders' answer: the chip asks `gz-levelbuild`'s
//! [`REBUILD_STATUS`] command now and then (never in a frame: the answer is cached) and shows
//! only while something does.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Align2, CornerRadius, Id, Sense, Ui, Vec2, pos2};
use gz_app::{DocCx, EditorSettings, ShellBuilder};
use gz_editor::{Args, Editor, Value};
use gz_scene::SceneService;
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Tone, audit, palette};
use parking_lot::Mutex;

use crate::toolbar;

/// The command that says what the scene's edits left for the level builders:
/// `{collision, ai_map, game_graph}`, each `true` while it needs building.
pub const REBUILD_STATUS: &str = "level_rebuild_status";

/// The chip's items: the name, the field of [`REBUILD_STATUS`] and the toolbar action that
/// builds it.
const ITEMS: [(&str, &str, &str); 3] = [
    ("Collision", "collision", "build_cform"),
    ("AI Map", "ai_map", "build_aimap"),
    ("Game Graph", "game_graph", "build_ggraph"),
];

/// What the last ask answered.
#[derive(Default)]
struct RebuildCache {
    state: Mutex<(Option<Instant>, [bool; 3], bool)>,
}

/// What needs rebuilding, as last answered; asks again when the answer is a second old.
fn pending(editor: &Arc<Editor>) -> [bool; 3] {
    if !editor.registry().has(REBUILD_STATUS) || editor.project_opt().is_none() {
        return [false; 3];
    }
    let cache = editor.services.get_or_init(RebuildCache::default);
    let mut st = cache.state.lock();
    let stale = st.0.is_none_or(|t| t.elapsed() >= Duration::from_secs(1));
    if stale && !st.2 {
        st.2 = true;
        st.0 = Some(Instant::now());
        let ed = editor.clone();
        editor.exec.spawn(async move {
            let reply = ed.call(REBUILD_STATUS, Args::new(), gz_base::Origin::Mcp).await;
            let flag = |k: &str| reply.get(k).and_then(Value::as_bool).unwrap_or(false);
            let now = [flag("collision"), flag("ai_map"), flag("game_graph")];
            let c = ed.services.get_or_init(RebuildCache::default);
            let mut st = c.state.lock();
            let changed = st.1 != now;
            st.1 = now;
            st.2 = false;
            drop(st);
            if changed {
                ed.exec.wake().ping();
            }
        });
    }
    st.1
}

/// Whether the Rebuild chip is up (`UIMainForm::RebuildPending`).
#[must_use]
pub fn rebuild_pending(editor: &Arc<Editor>) -> bool {
    let objects = editor
        .project_opt()
        .and_then(|s| s.services.get::<SceneService>())
        .is_some_and(|svc| !svc.store.snapshot().objects.is_empty());
    objects && pending(editor).iter().any(|p| *p)
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
    let mut label = String::new();
    for (i, (name, _, _)) in ITEMS.iter().enumerate() {
        if items.get(i).copied().unwrap_or(false) {
            label.push_str(if label.is_empty() { "Rebuild: " } else { " \u{b7} " });
            label.push_str(name);
        }
    }
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
    let resp = resp.on_hover_text(
        "The scene changed since these were built. The game needs them rebuilt before it can play the level.",
    );
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
                ITEMS.iter().map(|(n, _, _)| text::width(&ctx, &Font::Body.id(), n)).fold(0.0, f32::max);
            for (i, (name, _, action)) in ITEMS.iter().enumerate() {
                if !items.get(i).copied().unwrap_or(false) {
                    continue;
                }
                ui.horizontal(|ui| {
                    ui.allocate_ui(Vec2::new(name_w + 24.0, gz_ui::metrics::FRAME), |ui| {
                        text::label(ui, name, Font::Body, p.text, Clip::End)
                    });
                    let Some(a) = toolbar::action(action) else { return };
                    let why = "A compiler or the game is running.";
                    if gz_ui::Button::new("Build").enabled(!busy, why).show(ui).clicked() {
                        toolbar::run(&editor, a, Args::new());
                        egui::Popup::close_id(&ctx, popup);
                    }
                });
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
