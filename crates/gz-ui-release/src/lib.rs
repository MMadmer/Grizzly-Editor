//! The windows that put a module in front of players (docs/UI_DESIGN.md, section 6, "Publish
//! Release", "Releases", "Submit to Mod Browser").
//!
//! These are faces of commands that already exist: every button runs the domain path the command
//! runs (`gz_publish::Publisher::start`, `gz_publish::ReleaseList::start`,
//! `gz_catalog::Catalog::start`, the author key's and the accounts' own calls), nothing here
//! talks to the network or writes a file of its own, and no check is made optional by the UI. No
//! secret is drawn except the device code the author must type. A frame only reads what the
//! services hold; whatever touches a file or the network runs on the pools and lands in a slot
//! the next frame reads.
//!
//! - [`install`] - the Publish Release dialog (`publish_release`, with `dlg.edit_release` and the
//!   publish result), the Releases tab (document kind `releases`), the Submit to Mod Browser
//!   dialog (`submit`, with the sheet `dlg.catalog_withdraw`), the toolbar's Publish Release and
//!   Releases, the Mod menu's Publish Release..., Releases and Submit to Mod Browser...
//! - The author key row of Publish Release is `gz_ui_mod::KeyLine` (`EditorCatalog::DrawKeyLine`),
//!   the one Package Release draws; the Mod menu's rows go where `gz_ui_mod::mod_menu_slot` puts
//!   them.
//!
//! Widgets the kit lacks, local to this crate (`local`): `local_secret_field` (a one-line field
//! whose text is never drawn), `local_secret_row` (that field and its button on one line),
//! `local_code` (the device code in Title-sized Mono), `local_card_title` (a card's title with a
//! badge and a button on its line), `local_fold` (a folded block outside a property grid),
//! `local_table_head` (a sortable table's header with column lines in the header only).

mod local;
mod publish;
mod releases;
mod submit;

use std::cell::RefCell;
use std::rc::Rc;

use gz_app::{DialogDesc, MenuContent, MenuEntry, OpenRequest, ShellBuilder, UiCommand};
use gz_editor::{Editor, SmolStr};
use gz_ui::{Asker, Icon, Kind};

/// The Publish Release dialog: its MCP id and its window.
pub const PUBLISH_ID: &str = "publish_release";
pub const PUBLISH_WINDOW: &str = "Publish Release###dlg.publish_release";
/// The edit dialog of a release, raised from the Releases tab or by `mod_releases select show=edit`.
pub const EDIT_WINDOW: &str = "Edit Release###dlg.edit_release";
/// The Submit to Mod Browser dialog: its MCP id and its window.
pub const SUBMIT_ID: &str = "submit";
pub const SUBMIT_WINDOW: &str = "Submit to Mod Browser###dlg.catalog_submit";
/// The withdrawal's question, a sheet of the Submit dialog.
pub const WITHDRAW_WINDOW: &str = "Withdraw from Mod Browser###dlg.catalog_withdraw";
/// The Releases tab: its document kind and its id.
pub const RELEASES_KIND: &str = "releases";

/// The shortcut ids of the Mod menu's rows (`EditorPreferences.cpp`).
const CMD_PUBLISH: &str = gz_ui_mod::menu::PUBLISH;
const CMD_RELEASES: &str = gz_ui_mod::menu::RELEASES;
const CMD_CATALOG: &str = gz_ui_mod::menu::CATALOG;

/// What the Publish dialog, the edit dialog, the Releases tab, the toolbar and the menu share. UI
/// thread only.
pub(crate) type Hub = Rc<RefCell<publish::ReleaseUi>>;

/// Who answers a kit question raised from the window.
pub(crate) fn asker(editor: &Editor) -> Asker {
    if editor.flags.unattended { Asker::Unattended } else { Asker::Ui }
}

/// A GitHub page, a VirusTotal report: opened in the browser only when it is one of those. An
/// address that came over the network is never opened as it stands.
pub(crate) fn open_page(ctx: &egui::Context, url: &str, web: &str) {
    if page_ok(url, web) {
        ctx.open_url(egui::OpenUrl::new_tab(url));
    }
}

/// GitHub's own pages (or the QA mock's, `web`) and VirusTotal's report pages.
pub(crate) fn page_ok(url: &str, web: &str) -> bool {
    let under = |base: &str| {
        !base.is_empty()
            && url.len() > base.len()
            && url.starts_with(base)
            && url.as_bytes().get(base.len()) == Some(&b'/')
    };
    under("https://github.com") || under("https://www.virustotal.com/gui/file") || under(web)
}

/// A reason GitHub, the network or a job gave, without its own period: the end of a sentence.
pub(crate) fn reason(text: &str) -> &str {
    text.trim_end_matches(['.', ' '])
}

/// A reason as a sentence of its own: a capital and a period.
pub(crate) fn sentence(text: &str) -> String {
    let r = reason(text);
    let mut chars = r.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

/// Plugs the windows into the shell.
pub fn install(b: &mut ShellBuilder) {
    let hub: Hub = Rc::new(RefCell::new(publish::ReleaseUi::default()));

    // the Publish Release dialog; it also draws the edit dialog and the publish result, which
    // are the release window's own and never up together with it
    let (open_hub, draw_hub) = (hub.clone(), hub.clone());
    b.dialog(DialogDesc {
        id: SmolStr::new(PUBLISH_ID),
        window: SmolStr::new(PUBLISH_WINDOW),
        needs_project: true,
        open: Box::new(move |cx| open_hub.borrow_mut().request_publish(cx)),
        draw: Box::new(move |ui, cx| draw_hub.borrow_mut().draw(ui, cx)),
    });

    let submit = Rc::new(RefCell::new(submit::SubmitUi::default()));
    let (open_submit, draw_submit) = (submit.clone(), submit);
    b.dialog(DialogDesc {
        id: SmolStr::new(SUBMIT_ID),
        window: SmolStr::new(SUBMIT_WINDOW),
        needs_project: true,
        open: Box::new(move |cx| {
            open_submit.borrow_mut().request(cx.editor);
            Ok(())
        }),
        draw: Box::new(move |ui, cx| draw_submit.borrow_mut().draw(ui, cx)),
    });

    let doc_hub = hub.clone();
    b.document(RELEASES_KIND, move |editor, _req| {
        Ok(Box::new(releases::ReleasesDoc::new(editor, doc_hub.clone())))
    });

    b.command(
        UiCommand::new(CMD_PUBLISH, "Mod", "Publish Release", Icon::Publish, |cx| {
            cx.requests.dialog(PUBLISH_ID)
        })
        .enabled(|ed: &Editor, _| ed.project_opt().is_some()),
    );
    let cmd_hub = hub.clone();
    b.command(
        UiCommand::new(CMD_RELEASES, "Mod", "Releases", Icon::Releases, move |cx| {
            cmd_hub.borrow_mut().refresh_wanted = true;
            cx.requests.open(OpenRequest::kind(RELEASES_KIND));
        })
        .enabled(|ed: &Editor, _| ed.project_opt().is_some()),
    );
    b.command(
        UiCommand::new(CMD_CATALOG, "Mod", "Submit to Mod Browser", Icon::Globe, |cx| {
            cx.requests.dialog(SUBMIT_ID)
        })
        .enabled(|ed: &Editor, _| ed.project_opt().is_some()),
    );
    // Mod: ... | Package Release..., Publish Release..., Releases | Submit to Mod Browser...
    for (command, label) in [
        (CMD_PUBLISH, Some("Publish Release...")),
        (CMD_RELEASES, None),
        (CMD_CATALOG, Some("Submit to Mod Browser...")),
    ] {
        let (group, order) = gz_ui_mod::mod_menu_slot(command).unwrap_or((3, 9));
        b.menu_entry(MenuEntry {
            menu: SmolStr::new("Mod"),
            submenu: None,
            group,
            order,
            content: MenuContent::Command { command: SmolStr::new(command), label: label.map(SmolStr::new) },
        });
    }

    // the toolbar, after Build Mod: [Publish Release] [Releases]
    let tb_hub = hub;
    b.project_toolbar(20, move |tb, cx| {
        let publisher = gz_publish::Publisher::of(cx.editor);
        let state = publisher.button_text();
        let running = publisher.running();
        // the label keeps its width: what a running release does is in the tip, and the button is lit meanwhile
        let mut tip = if running { format!("{state}.\nClick to see the job.\n") } else { String::new() };
        tip.push_str("Package the module and publish it as a GitHub release.\nPlayers receive it through the game's Mods menu.");
        let queued = tb_hub.borrow().publish_queued();
        if tb.action("publish_release", Icon::Publish, "Publish Release", &tip, !queued, running, Kind::Ghost) {
            if running {
                cx.requests.task_card(true);
            } else {
                cx.requests.dialog(PUBLISH_ID);
            }
        }
        if cx.active_tab != RELEASES_KIND
            && tb.action("releases", Icon::Releases, "Releases", "The releases already on GitHub.\nDeletes one together with its git tag.", true, false, Kind::Ghost)
        {
            tb_hub.borrow_mut().refresh_wanted = true;
            cx.requests.open(OpenRequest::kind(RELEASES_KIND));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{page_ok, reason, sentence};

    #[test]
    fn pages_are_github_or_virustotal() {
        assert!(page_ok("https://github.com/me/od/releases/tag/v1.0.0", ""));
        assert!(page_ok("https://www.virustotal.com/gui/file/ab", ""));
        assert!(page_ok("http://127.0.0.1:9/me/od", "http://127.0.0.1:9"));
        assert!(!page_ok("https://github.com.evil/x", ""));
        assert!(!page_ok("https://example.com/", ""));
        assert!(!page_ok("http://127.0.0.1:9", "http://127.0.0.1:9"));
    }

    #[test]
    fn reasons_and_sentences() {
        assert_eq!(reason("timed out. "), "timed out");
        assert_eq!(sentence("the list is busy."), "The list is busy.");
        assert_eq!(sentence(""), "");
    }
}
