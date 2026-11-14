//! Link Game: an M page card on the Start page, drawn instead of the editor's UI only while no
//! valid game is known at all - neither the editor's default (found by itself: the game this
//! editor knew, the installed game, a game beside the editor) nor one of the open project's own.
//! It offers every place the editor looked at as a pick list, beside the folder field, so nothing
//! that can be worked out is asked: [Close Project] [Link]. Link sets the project's own game on
//! this machine (`game.link scope=project`); the editor finds it again for the next project.
//!
//! The Mod menu's Link Game... opens Preferences > Game, where the project's own game is changed
//! or given back (Use Default Game).

use std::sync::Arc;

use egui::Ui;
use gz_app::{Action, DocCx, Enabled, PageCard, ShellBuilder, contract};
use gz_editor::{Editor, SmolStr};
use gz_ui::card::{CardAction, card_footer, page, page_card};
use gz_ui::grid::{PathAction, path_field};
use gz_ui::pick::{PickItem, PickList};
use gz_ui::text;
use gz_ui::{Icon, Kind, Tone};
use parking_lot::Mutex;
use serde_json::Value;

use crate::folder::pick_folder;
use crate::reply::{args, call_then, error_text, ok, post_ui, s};

/// One place the editor looked for the game.
#[derive(Clone, Debug, Default)]
struct Place {
    root: String,
    found: String,
    reason: String,
}

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
    /// The project the card was filled for: another one starts again.
    project: String,
    /// The places the editor looked at, as `game.info candidates=true` answered.
    places: Vec<Place>,
    places_asked: bool,
}

/// The card's service.
#[derive(Default)]
pub struct LinkCard {
    state: Mutex<LinkState>,
}

/// What the hint says of a game folder: the editor only reads it.
const GAME_OK: &str = "A Dead Air install: fsgame.ltx and its data are there.";

/// The Mod menu's row.
pub const LINK_GAME: &str = "mod.link_game";

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
    // the project area's groups of Preferences (Game, Library, Project Files)
    crate::game_prefs::install(b);
    b.action(
        Action::new(LINK_GAME, "Link Game...", "Mod", "app.prefs")
            .icon(Icon::GameFolder)
            .args(|_| gz_app::args(&[("action", "show"), ("page", "Game")]))
            .enabled(|cx| Enabled::when(cx.editor.project_opt().is_some(), "Open a project first.")),
    );
    b.menu_action("Mod", 0, 5, LINK_GAME, None);
}

fn places_of(reply: &Value) -> Vec<Place> {
    reply
        .get("candidates")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|c| Place {
                    root: c.get("root").and_then(Value::as_str).unwrap_or_default().replace('/', "\\"),
                    found: c.get("found").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    reason: c.get("reason").and_then(Value::as_str).unwrap_or_default().to_owned(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn found_words(key: &str) -> &'static str {
    match key {
        "installed" => "the installer's entry",
        "beside_editor" => "beside the editor",
        _ => "linked before",
    }
}

fn draw(card: &Arc<LinkCard>, ui: &mut Ui, editor: &Arc<Editor>) {
    let info = gz_app::project::project_info(editor);
    let mut st = card.state.lock();
    if st.project != info.folder {
        // a project of its own: the field starts empty, the places are looked at again
        st.project.clone_from(&info.folder);
        st.path.clear();
        st.error.clear();
        st.checked = String::from("\u{0}");
        st.places.clear();
        st.places_asked = false;
    }
    if !st.places_asked {
        st.places_asked = true;
        let (ed, c) = (editor.clone(), card.clone());
        call_then(editor, "game.info", args([("candidates", Value::Bool(true))]), move |reply, _| {
            let places = places_of(reply);
            let mut st = c.state.lock();
            // the first place that is there at all is a good start for the field
            if st.path.is_empty()
                && let Some(p) = places.first()
            {
                st.path.clone_from(&p.root);
            }
            st.places = places;
            drop(st);
            ed.exec.wake().ping();
        });
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
    let mut picked_place = None;
    let live_scene = gz_project::ProjectHooks::of(editor).live_scene(editor);
    page(ui, contract::WINDOW_START_PAGE, |ui| {
        page_card(ui, "link_game", "Link Game", gz_ui::metrics::DIALOG_M, |ui| {
            text::prose(
                ui,
                "The editor found no Dead Air Refined install on this computer. Pick the folder that holds fsgame.ltx and database\\ or gamedata\\: the editor only reads it and never writes into it.",
            );
            ui.add_space(12.0);
            if !st.places.is_empty() {
                text::secondary(ui, "Places the Editor Looked At");
                let items: Vec<PickItem> = st
                    .places
                    .iter()
                    .map(|p| {
                        let detail = if p.reason.is_empty() {
                            format!("A game ({})", found_words(&p.found))
                        } else {
                            format!("{} ({})", p.reason, found_words(&p.found))
                        };
                        PickItem::new(p.root.clone(), p.root.clone()).icon(Icon::GameFolder).detail(detail)
                    })
                    .collect();
                let current = st.path.clone();
                let shown = PickList::new("link_game.places", &items)
                    .without_search()
                    .current(&current)
                    .max_height(gz_ui::units::em(9.0))
                    .width(ui.available_width())
                    .show(ui);
                picked_place = shown.item;
                ui.add_space(8.0);
            }
            text::secondary(ui, "Game Folder");
            if path_field(ui, "link_game.game_root", &mut st.path, true) == PathAction::Browse {
                browse = true;
            }
            ui.add_space(2.0);
            if st.path.is_empty() {
                text::status(ui, Tone::Muted, "Browse for the game folder.");
            } else if st.hint.is_empty() {
                gz_ui::state::busy_line(ui, "Looking at the folder...");
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
                // a game that went away mid-session leaves a scene open behind this card: it is
                // not closed from here
                CardAction::new("Close Project", Kind::Secondary).enabled(!live_scene, "A scene is open. Link the game to get back to it; File > Project > Switch Project... closes the project."),
                CardAction::new("Link", Kind::Primary).default_action().enabled(st.hint_ok && !st.linking, why),
            ];
            clicked = card_footer(ui, &format!("Project: {}", info.name), Tone::Muted, &actions);
        });
    });

    if let Some(root) = picked_place.and_then(|i| st.places.get(i)).map(|p| p.root.clone()) {
        st.path = root;
    }
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
            let a = args([("path", s(&st.path)), ("scope", s("project"))]);
            call_then(editor, "game.link", a, move |reply, _| {
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
        Some(0) => editor.post("project.close", gz_editor::Args::new()),
        _ => {}
    }
}
