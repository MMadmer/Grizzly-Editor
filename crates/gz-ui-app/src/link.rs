//! Link Game: an M page card on the Start page, drawn instead of the editor's UI while the open
//! project has no valid game - [Close Project] [Link]. The only ways on are a valid link and
//! Close Project, which goes back to the project list (`EditorProject::DrawGameLink`).

use std::sync::Arc;

use egui::Ui;
use gz_app::{DocCx, PageCard, ShellBuilder, contract};
use gz_editor::{Editor, SmolStr};
use gz_project::Project;
use gz_ui::card::{CardAction, card_footer, page, page_card};
use gz_ui::grid::{PathAction, path_field};
use gz_ui::text;
use gz_ui::{Kind, Tone};
use parking_lot::Mutex;

use crate::folder::pick_folder;
use crate::reply::{args, call_then, error_text, ok, post_ui, s};

/// What the card says while a game folder is typed or picked.
#[derive(Default)]
struct LinkState {
    path: String,
    /// The text the hint was worked out for.
    checked: String,
    check_gen: u64,
    hint: String,
    hint_ok: bool,
    /// The last link that failed.
    error: String,
    linking: bool,
    /// The project the card was filled for: another one starts from its own `game_root`.
    project: String,
}

/// The card's service.
#[derive(Default)]
pub struct LinkCard {
    state: Mutex<LinkState>,
}

/// What the hint says of a game folder: the editor only reads it.
const GAME_OK: &str = "A Dead Air install: fsgame.ltx and its data are there.";

pub(crate) fn install(b: &mut ShellBuilder) {
    b.page_card(PageCard {
        id: SmolStr::new("link_game"),
        wants: Box::new(|editor: &Editor| {
            let info = gz_app::project::project_info(editor);
            info.active && !info.game_linked
        }),
        draw: Box::new(|ui: &mut Ui, cx: &mut DocCx<'_>| {
            let card = cx.editor.services.get_or_init(LinkCard::default);
            draw(&card, ui, cx.editor);
        }),
    });
}

fn draw(card: &Arc<LinkCard>, ui: &mut Ui, editor: &Arc<Editor>) {
    let info = gz_app::project::project_info(editor);
    let mut st = card.state.lock();
    if st.project != info.folder {
        // a project of its own: the field starts at the link it has (gone or broken)
        st.project.clone_from(&info.folder);
        st.path = Project::of(editor).map(|(_, p)| p.game_root()).unwrap_or_default();
        st.error.clear();
        st.checked = String::from("\u{0}");
    }
    // the hint follows the text, looked at again only when the text changed
    if st.path != st.checked {
        st.checked = st.path.clone();
        st.hint.clear();
        st.hint_ok = false;
        st.check_gen += 1;
        let (generation, folder) = (st.check_gen, st.path.clone());
        let (ed, c) = (editor.clone(), card.clone());
        let _task = editor.exec.spawn(async move {
            let probe = folder.clone();
            let found = ed.exec.io(move || gz_project::looks_like_game(&probe)).await;
            post_ui(&ed, move |_| {
                let mut st = c.state.lock();
                if st.check_gen == generation {
                    st.hint_ok = found.is_ok();
                    st.hint = match found {
                        Ok(()) => GAME_OK.to_owned(),
                        Err(why) => why.to_owned(),
                    };
                }
            });
        });
    }

    let mut clicked = None;
    let mut browse = false;
    let live_scene = gz_project::ProjectHooks::of(editor).live_scene(editor);
    page(ui, contract::WINDOW_START_PAGE, |ui| {
        page_card(ui, "link_game", "Link Game", gz_ui::metrics::DIALOG_M, |ui| {
            text::prose(
                ui,
                "Pick the Dead Air Refined install this project builds for: the folder that holds fsgame.ltx and database\\ or gamedata\\. The editor only reads it and never writes into it.",
            );
            ui.add_space(12.0);
            text::secondary(ui, "Game Folder");
            if path_field(ui, "link_game.game_root", &mut st.path, true) == PathAction::Browse {
                browse = true;
            }
            ui.add_space(2.0);
            if st.path.is_empty() {
                text::status(ui, Tone::Muted, "No folder selected.");
            } else if st.hint.is_empty() {
                text::status(ui, Tone::Muted, "Looking at the folder...");
            } else {
                let hint = st.hint.clone();
                text::status(ui, if st.hint_ok { Tone::Success } else { Tone::Error }, &hint);
            }
            if !st.error.is_empty() {
                let e = format!("Link failed: {}", st.error);
                text::status(ui, Tone::Error, &e);
            }
            let why = if st.path.is_empty() {
                "Pick the game folder first."
            } else if st.linking {
                "The game is being linked."
            } else {
                st.hint.as_str()
            };
            let actions = [
                // a link that went away mid-session leaves a scene open behind this card: it is
                // not closed from here
                CardAction::new("Close Project", Kind::Secondary).enabled(!live_scene, "A scene is open. Link the game to get back to it; File > Project > Switch Project... closes the project."),
                CardAction::new("Link", Kind::Primary).default_action().enabled(st.hint_ok && !st.linking, why),
            ];
            clicked = card_footer(ui, &format!("Project: {}", info.name), Tone::Muted, &actions);
        });
    });

    if browse {
        let c = card.clone();
        let start = st.path.clone();
        pick_folder(editor, "Game Folder", &start, move |folder, _| c.state.lock().path = folder);
    }
    match clicked {
        Some(1) => {
            st.linking = true;
            st.error.clear();
            let c = card.clone();
            call_then(editor, "game_link", args([("path", s(&st.path))]), move |reply, _| {
                let mut st = c.state.lock();
                st.linking = false;
                if ok(reply) {
                    st.checked = String::from("\u{0}");
                } else {
                    st.error = error_text(reply);
                }
            });
        }
        // the project's tabs come back from its session when it opens again
        Some(0) => editor.post("project_close", gz_editor::Args::new()),
        _ => {}
    }
}
