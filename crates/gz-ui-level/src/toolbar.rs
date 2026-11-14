//! The Level toolbar: `[Save] [Build Level v] [Play v] [Stop]` and one More for the rare ones,
//! labelled, lowest priority first. Every button is an action ([`crate::actions`]); Build Level's
//! menu lists the steps of the build with their state (fresh, stale and why, failed) to build
//! one, Rebuild All and Advanced; Play's menu plays clean or without building.

use gz_app::{DocCx, toolbar_button};
use gz_ui::{Icon, Kind, Toolbar, Tx};

use crate::actions as a;
use crate::menus;

/// The tip of a split's main half: what it does, its keys, or why it is off.
fn tip_of(cx: &DocCx<'_>, id: &str) -> (bool, String, &'static str, Icon) {
    let Some(actions) = gz_app::Actions::of(cx.editor) else { return (false, String::new(), "", Icon::None) };
    let table = actions.table.read();
    let Some(action) = table.get(id).copied() else { return (false, String::new(), "", Icon::None) };
    let keys = table.keys_label(id);
    drop(table);
    let on = actions.with_cx(cx.editor, |acx| (action.enabled)(acx));
    // the tip is put together from words already in the window's language
    let title = gz_ui::i18n::t(cx.ctx, action.title);
    let tip = match (on, keys.is_empty()) {
        (gz_app::Enabled::Off(why), _) => gz_ui::i18n::t(cx.ctx, why),
        (gz_app::Enabled::On, true) => title,
        (gz_app::Enabled::On, false) => format!("{title} ({keys})"),
    };
    (on.on(), tip, action.title, action.icon.unwrap_or(Icon::None))
}

/// A split button of an action: the click runs it, the chevron opens `menu`.
fn split(
    tb: &mut Toolbar<'_, '_>,
    cx: &mut DocCx<'_>,
    id: &'static str,
    menu: impl FnOnce(&mut egui::Ui, &mut DocCx<'_>),
) {
    let (on, tip, title, icon) = tip_of(cx, id);
    if title.is_empty() {
        return;
    }
    let pressed = tb.split(id, icon, title, Tx::raw(&tip), on, Kind::Ghost, |ui| menu(ui, cx));
    if pressed {
        cx.requests.action(id);
    }
}

/// Draws the Level toolbar.
pub fn draw(tb: &mut Toolbar<'_, '_>, cx: &mut DocCx<'_>) {
    tb.group(3);
    toolbar_button(tb, cx, "file.save", Kind::Ghost, None);
    tb.group(2);
    split(tb, cx, a::BUILD_LEVEL, |ui, cx| {
        menus::step_rows(ui, cx);
        ui.separator();
        gz_app::menu_row(ui, cx, a::REBUILD_ALL, None);
        gz_app::chrome::local_submenu(ui, "Advanced", Icon::None, true, |ui| {
            menus::rows(ui, cx, menus::ADVANCED)
        });
    });
    split(tb, cx, a::PLAY, |ui, cx| {
        menus::rows(ui, cx, &[&[a::PLAY_CLEAN, a::PLAY_WITHOUT_BUILDING]]);
    });
    toolbar_button(tb, cx, a::STOP, Kind::Ghost, None);
    tb.group(1);
    tb.menu(
        "level_more",
        Icon::More,
        "More",
        "Game configs, the game's folder, exports",
        Some("level_more"),
        true,
        |ui| {
            menus::rows(ui, cx, &[&[a::RELOAD_CONFIGS, a::GAME_FOLDER], &[a::VALIDATE, a::SHOW_PROBLEMS]]);
        },
    );
}
