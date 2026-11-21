//! The editor's own updates in the window (docs/UPDATES.md, sections 3 - 5; docs/UI_DESIGN.md,
//! "Updates"): the status bar chip, the dialog `Update###dlg.update`, Help > Check for Updates...,
//! the notes after a check and after an update, and the Updates group of Preferences > General.
//!
//! Everything here reads the updater's snapshot (`gz_update::UpdateService`) and runs `update.*`
//! commands: the window and MCP take one path.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use egui::{ScrollArea, Ui};
use gz_app::chrome::local_status_chip;
use gz_app::{Action, DialogDesc, DocCx, Enabled, PrefGroup, PrefKind, PrefRow, ShellBuilder};
use gz_editor::{Args, Editor, SmolStr};
use gz_ui::dialog::{Dialog, DialogSize};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Kind, Tone, Tx, palette, tr};
use gz_update::{State, UpdateService};

use crate::local::size_tx;
use serde_json::json;

/// `ui.dialog open id=update`.
pub const DIALOG_ID: &str = "update";
pub const DIALOG_WINDOW: &str = "Update###dlg.update";
/// Help > Check for Updates...: a check, and the dialog that follows it.
pub const CHECK_ACTION: &str = "help.check_updates";
/// The dialog with what the updater knows (a note's Details and What's New).
pub const SHOW_ACTION: &str = "help.update";
/// What a development build says wherever the updater would be.
const DEV_BUILD: &str = "This is a development build: it does not update itself.";

/// The window's side of the updater. UI thread only.
#[derive(Default)]
struct UpdateUi {
    open: bool,
    /// The notes of the update this start came from were looked at.
    whats_new_seen: bool,
    /// Showing the notes of the update this start came from.
    whats_new: bool,
    /// Frames since Restart and Update asked the editor to exit: a cancelled exit leaves Update on
    /// Exit, not a restart at some later exit.
    exit_frames: Option<u32>,
}

fn args(pairs: &[(&str, serde_json::Value)]) -> Args {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
}

/// Starts a check from the window unless an update job runs.
fn check_now(editor: &Arc<Editor>) {
    if UpdateService::of(editor).is_some_and(|s| s.install.is_some())
        && editor.jobs.running_in("update").is_none()
    {
        editor.post("update.check", args(&[("wait", json!(false))]));
    }
}

/// A line of the notes with its Markdown set aside: emphasis, code marks, a link's address.
fn plain(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    // `[text](address)` reads as its text
    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find("](").map(|c| open + c) else { break };
        let Some(end) = rest[close..].find(')').map(|e| close + e) else { break };
        out.push_str(&rest[..open]);
        out.push_str(&rest[open + 1..close]);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out.replace("**", "").replace("__", "").replace("~~", "").replace('`', "")
}

/// The release notes in the window's language (`gz_update::notes`), as plain paragraphs: a `#` line
/// as a heading, a list item (`-`, `*`, `+`, `1.`) as a bullet or its number, the rest as text. A
/// line written any other way is still shown, as text.
fn notes(ui: &mut Ui, body: &str) {
    let p = palette(ui.ctx());
    let lang = match gz_ui::i18n::lang(ui.ctx()) {
        gz_ui::Lang::Ru => "ru",
        _ => "en",
    };
    let shown = gz_update::notes::for_language(body, lang);
    for raw in shown.lines() {
        let line = raw.trim();
        let nested = raw.len() - raw.trim_start().len() >= 2;
        let indent = if nested { "      " } else { "" };
        let numbered = line
            .find(['.', ')'])
            .filter(|&at| at > 0 && at <= 3 && line[..at].chars().all(|c| c.is_ascii_digit()))
            .filter(|&at| line[at + 1..].starts_with(' '));
        if line.is_empty() {
            ui.add_space(4.0);
        } else if line.starts_with('#') {
            ui.add_space(2.0);
            text::label(
                ui,
                Tx::raw(plain(line.trim_start_matches('#').trim())),
                Font::Strong,
                p.text,
                Clip::End,
            );
        } else if let Some(item) =
            line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")).or_else(|| line.strip_prefix("+ "))
        {
            let mark = if nested { "\u{25E6}" } else { "\u{2022}" };
            text::prose(ui, Tx::raw(format!("{indent}{mark}  {}", plain(item.trim()))));
        } else if let Some(at) = numbered {
            text::prose(ui, Tx::raw(format!("{indent}{}  {}", &line[..=at], plain(line[at + 1..].trim()))));
        } else if let Some(quote) = line.strip_prefix('>') {
            text::prose(ui, Tx::raw(format!("\u{201C}{}\u{201D}", plain(quote.trim()))));
        } else {
            text::prose(ui, Tx::raw(plain(line)));
        }
    }
}

impl UpdateUi {
    fn chip(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let Some(svc) = UpdateService::of(cx.editor) else { return };
        // a note that the dialog says more about carries its button: Details, What's New
        for t in svc.take_toasts() {
            match (t.error, t.whats_new) {
                // the updater's sentences are its own English, translated by their templates
                (true, _) => gz_ui::state::toast(cx.ctx, Tone::Error, Tx::ui(t.text)),
                (false, true) => {
                    gz_ui::state::toast_with(cx.ctx, Tone::Success, Tx::ui(t.text), "What's New", SHOW_ACTION)
                }
                (false, false) => gz_ui::state::toast_with(
                    cx.ctx,
                    Tone::Info,
                    Tx::ui(t.text),
                    Tx::ctx("update", "Details"),
                    SHOW_ACTION,
                ),
            }
        }
        self.watch_exit(cx, &svc);
        let status = svc.status(cx.editor);
        let latest = status["latest"].as_str().unwrap_or("").to_owned();
        let pct = status["download"]["total"]
            .as_u64()
            .filter(|t| *t > 0)
            .map(|t| (status["download"]["done"].as_u64().unwrap_or(0) * 100 / t).min(100))
            .unwrap_or(0);
        let (icon, tone, label): (Icon, Option<Tone>, Tx<'static>) = match svc.state() {
            State::Available | State::Manual => {
                (Icon::Refresh, Some(Tone::Info), tr!("Update {version}", version = latest.clone()))
            }
            State::Failed => {
                (Icon::Refresh, Some(Tone::Error), tr!("Update {version}", version = latest.clone()))
            }
            State::Downloading => (Icon::Spinner, None, tr!("Updating {pct}%", pct = pct)),
            State::Waiting => (Icon::Refresh, Some(Tone::Warning), "Waiting".into()),
            State::Ready => (Icon::Refresh, Some(Tone::Info), "Restart to Update".into()),
            State::Applying => (Icon::Spinner, None, "Updating".into()),
            // the start after an update offers its notes on its note (What's New)
            _ => return,
        };
        let tip = Tx::ui(svc.text());
        if local_status_chip(ui, icon, tone, label, tip, self.open).clicked() {
            self.whats_new = svc.state() == State::Idle && svc.updated().is_some();
            self.open = true;
            if self.whats_new {
                self.whats_new_seen = true;
            }
        }
    }

    /// After Restart and Update asked to exit: an exit that did not come (the unsaved-work
    /// question was cancelled) turns the restart back into Update on Exit.
    fn watch_exit(&mut self, cx: &mut DocCx<'_>, svc: &UpdateService) {
        let Some(frames) = self.exit_frames.as_mut() else { return };
        *frames += 1;
        cx.ctx.request_repaint();
        if cx.editor.quit_token().is_cancelled() {
            self.exit_frames = None;
        } else if *frames > 5 && gz_ui::dialog::modal_on_top(cx.ctx).is_none() {
            svc.set_restart_asked(cx.editor, false);
            self.exit_frames = None;
        }
    }

    fn draw(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        if !self.open {
            return;
        }
        let editor = cx.editor.clone();
        let Some(svc) = UpdateService::of(&editor) else {
            self.open = false;
            return;
        };
        let state = svc.state();
        let status = svc.status(&editor);
        let have = status["version"].as_str().unwrap_or("").to_owned();
        let latest = status["latest"].as_str().unwrap_or("").to_owned();
        let updated = svc.updated();
        let whats_new = self.whats_new && state == State::Idle;
        let headline: Tx<'static> = match state {
            _ if whats_new => {
                tr!("Updated to {version}.", version = updated.as_ref().map_or(have.clone(), |u| u.1.clone()))
            }
            State::Available | State::Failed | State::Downloading | State::Waiting => tr!(
                "Grizzly Editor {latest} is available. You have {have}.",
                latest = latest.clone(),
                have = have.clone()
            ),
            State::Ready => tr!(
                "Grizzly Editor {latest} is ready to install. You have {have}.",
                latest = latest.clone(),
                have = have.clone()
            ),
            _ => Tx::ui(svc.text()),
        };
        let dl = &status["download"];
        let size_line = dl["bytes"].as_u64().map(|b| {
            let archive = dl["archive"].as_u64().unwrap_or(b);
            match state {
                State::Downloading | State::Waiting => tr!(
                    "{done} of {total} downloaded",
                    done = size_tx(dl["done"].as_u64().unwrap_or(0)),
                    total = size_tx(dl["total"].as_u64().unwrap_or(b))
                ),
                State::Ready => Tx::EMPTY,
                _ => tr!("{part} of {total} to download", part = size_tx(b), total = size_tx(archive)),
            }
        });
        let body_notes = if whats_new {
            updated.as_ref().map(|u| u.2.clone()).unwrap_or_default()
        } else if matches!(state, State::Idle | State::Off | State::Checking) {
            String::new()
        } else {
            status["notes"].as_str().unwrap_or("").to_owned()
        };
        let error = status["error"].as_str().filter(|_| state == State::Failed).map(|e| Tx::ui(e.to_owned()));
        let stage = editor.jobs.running_in("update").map(|j| j.stage).unwrap_or_default();
        let page = status["release_page"].as_str().unwrap_or("").to_owned();
        let mut open = self.open;
        let mut close = false;
        let ctx = ui.ctx().clone();
        Dialog::new(Tx::ctx("dlg.update", "Update"), "dlg.update", DialogSize::M).show(
            ui,
            &mut open,
            |ui| {
                let p = palette(ui.ctx());
                text::prose(ui, &headline);
                if let Some(line) = size_line.as_ref().filter(|l| !l.is_empty() && !whats_new) {
                    text::label(ui, line, Font::Body, p.text_secondary, Clip::End);
                }
                if let Some(e) = &error {
                    ui.add_space(4.0);
                    text::status(ui, Tone::Error, e);
                }
                if !body_notes.trim().is_empty() {
                    ui.add_space(8.0);
                    ScrollArea::vertical().id_salt("gz-ui-app.update.notes").max_height(260.0).show(
                        ui,
                        |ui| {
                            notes(ui, &body_notes);
                        },
                    );
                }
            },
            |f| match state {
                State::Checking => {
                    f.busy(if stage.is_empty() {
                        Tx::from("Looking for a newer version")
                    } else {
                        Tx::ui(&stage)
                    });
                    close = f.button("Hide", Kind::Secondary, false, true, "");
                }
                State::Downloading | State::Waiting => {
                    f.busy(if stage.is_empty() { Tx::from("Downloading") } else { Tx::ui(&stage) });
                    if f.button("Stop Updating", Kind::Secondary, false, true, "") {
                        editor.post("update.stop", Args::new());
                    }
                    close = f.button("Hide", Kind::Secondary, false, true, "");
                }
                State::Applying => f.busy(tr!("Updating to {version}...", version = latest.clone())),
                State::Available | State::Failed => {
                    if f.button("Download and Install", Kind::Primary, true, true, "") {
                        editor.post("update.download", args(&[("wait", json!(false))]));
                    }
                    close = f.button("Later", Kind::Secondary, false, true, "");
                }
                State::Ready => {
                    let busy = UpdateService::blocking_job(&editor);
                    let why = busy
                        .as_ref()
                        .map(|j| tr!("The update waits for {job} to end.", job = Tx::ui(j.clone())))
                        .unwrap_or_default();
                    if f.button("Restart and Update", Kind::Primary, true, busy.is_none(), why) {
                        // the documents close the usual way; Cancel there cancels the restart
                        svc.set_restart_asked(&editor, true);
                        self.exit_frames = Some(0);
                        cx.requests.exit();
                        close = true;
                    }
                    if f.button("Update on Exit", Kind::Secondary, false, true, "") {
                        svc.set_on_exit(&editor, true);
                        close = true;
                    }
                }
                State::Manual => {
                    if f.button("Open Release Page", Kind::Primary, true, !page.is_empty(), "") {
                        ctx.open_url(egui::OpenUrl::new_tab(&page));
                    }
                    close = f.button("Later", Kind::Secondary, false, true, "");
                }
                State::Idle | State::Off => {
                    close = f.button("Close", Kind::Primary, true, true, "");
                }
            },
        );
        // closing a ready update's dialog leaves Update on Exit
        if (!open || close) && state == State::Ready && !svc.restart_asked() {
            svc.set_on_exit(&editor, true);
        }
        self.open = open && !close;
        if !self.open {
            self.whats_new = false;
        }
    }

    fn open_dialog(&mut self, editor: &Arc<Editor>) {
        self.open = true;
        if let Some(svc) = UpdateService::of(editor)
            && svc.state() == State::Idle
            && svc.updated().is_some()
            && !self.whats_new_seen
        {
            self.whats_new = true;
            self.whats_new_seen = true;
        }
    }
}

/// The updater of an installation; `None` for a development build.
fn installed(editor: &Arc<Editor>) -> Option<Arc<UpdateService>> {
    UpdateService::of(editor).filter(|s| s.install.is_some())
}

/// The Updates group of Preferences > General: Check at Startup (kept by the updater itself,
/// `update.set`), the installed version, Check Now with what it found.
const UPDATE_ROWS: [PrefRow; 3] = [
    PrefRow {
        label: "Check at Startup",
        help: "Looks for a newer version once per start, after the window is up.",
        kind: PrefKind::Switch {
            get: |ed| installed(ed).is_some_and(|s| s.check_at_startup()),
            set: |ed, on| ed.post("update.set", args(&[("check_at_startup", json!(on))])),
            default: true,
        },
    },
    PrefRow {
        label: "Installed Version",
        help: "The version of this installation.",
        kind: PrefKind::ReadOnly {
            value: |ed| installed(ed).map(|s| Tx::raw(s.installed().to_string())).unwrap_or_default(),
        },
    },
    PrefRow {
        label: "Check Now",
        help: "Looks for a newer version now.",
        kind: PrefKind::Button {
            text: "Check Now",
            run: check_now,
            enabled: |ed| Enabled::when(ed.jobs.running_in("update").is_none(), "An update job is running."),
            // the updater's own English
            status: |ed| installed(ed).map(|s| Tx::ui(s.text())).unwrap_or_default(),
        },
    },
];

/// Plugs the chip, the dialog, Help > Check for Updates... and the Updates group of Preferences
/// into the shell.
pub(crate) fn install(b: &mut ShellBuilder) {
    let hub = Rc::new(RefCell::new(UpdateUi::default()));
    let chip = hub.clone();
    b.status_chip(5, move |ui, cx| chip.borrow_mut().chip(ui, cx));
    let (open_hub, draw_hub) = (hub.clone(), hub.clone());
    b.dialog(DialogDesc {
        id: SmolStr::new(DIALOG_ID),
        window: SmolStr::new(DIALOG_WINDOW),
        needs_project: false,
        open: Box::new(move |cx| {
            open_hub.borrow_mut().open_dialog(cx.editor);
            Ok(())
        }),
        draw: Box::new(move |ui, cx| draw_hub.borrow_mut().draw(ui, cx)),
    });
    // a check, then the dialog that says what it found (a development build's dialog says why
    // there is none)
    b.action(
        Action::new(CHECK_ACTION, "Check for Updates...", "Help", "update.check")
            .icon(Icon::Refresh)
            .args(|_| args(&[("wait", json!(false))]))
            .reply(|r| {
                r.editor.post("ui.dialog", args(&[("action", json!("open")), ("id", json!(DIALOG_ID))]))
            }),
    );
    b.action(
        Action::new(SHOW_ACTION, "Update Details", "Help", "ui.dialog")
            .icon(Icon::Info)
            .args(|_| args(&[("action", json!("open")), ("id", json!(DIALOG_ID))])),
    );
    b.menu_action("Help", 20, 0, CHECK_ACTION, None);
    b.pref_group(PrefGroup {
        page: "General",
        icon: Icon::Settings,
        section: "Updates",
        rows: &UPDATE_ROWS,
        note: Some(|ed| installed(ed).is_none().then(|| ("This Build", Tx::from(DEV_BUILD)))),
    });
}
