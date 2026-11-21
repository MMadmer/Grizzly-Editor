//! Feedback to the editor's author in the window (docs/REPORTS.md; docs/UI_DESIGN.md, "Feedback"): the
//! dialog `Send Feedback###dlg.feedback`, Help > Send Feedback..., and the toast a start after a crash
//! shows. Everything here runs `feedback.*` commands: the window and MCP take one path.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use egui::{Id, Ui};
use gz_app::{Action, DialogDesc, DocCx, ShellBuilder};
use gz_editor::{Args, Editor, SmolStr, Value};
use gz_feedback::FeedbackService;
use gz_ui::dialog::{Dialog, DialogSize};
use gz_ui::text::{self, Clip};
use gz_ui::{Button, Check, Font, Icon, Kind, Tone, Tx, palette, tr};
use parking_lot::Mutex;
use serde_json::json;

/// `ui.dialog open id=feedback`.
pub const DIALOG_ID: &str = "feedback";
pub const DIALOG_WINDOW: &str = "Send Feedback###dlg.feedback";
/// Help > Send Feedback..., and the crash toast's Send Report...
pub const ACTION: &str = "help.feedback";

const KINDS: [&str; 2] = ["Bug Report", "Suggestion"];

/// The crash the dialog offers: its name, when, and what it was.
#[derive(Clone)]
struct Offered {
    name: String,
    time: u64,
    summary: String,
}

/// The window's side of feedback. UI thread only.
#[derive(Default)]
struct FeedbackUi {
    open: bool,
    /// 0 a bug, 1 a suggestion.
    kind: usize,
    title: String,
    text: String,
    diagnostics: bool,
    /// The author set Attach Diagnostics: a change of the kind leaves it as it is.
    diagnostics_set: bool,
    crash: Option<Offered>,
    with_crash: bool,
    error: Option<String>,
    /// The answer of the send under way, once it is there.
    pending: Option<Arc<Mutex<Option<Value>>>>,
    /// The crash whose toast this run showed.
    toasted: Option<String>,
}

fn args(pairs: &[(&str, Value)]) -> Args {
    pairs.iter().map(|(k, v)| ((*k).to_owned(), v.clone())).collect()
}

/// `secs` since 1970 as the machine's local date and time.
fn local(secs: u64) -> String {
    let Ok(utc) = time::OffsetDateTime::from_unix_timestamp(i64::try_from(secs).unwrap_or(0)) else {
        return String::new();
    };
    let t = time::UtcOffset::current_local_offset().map_or(utc, |o| utc.to_offset(o));
    format!("{:04}-{:02}-{:02} {:02}:{:02}", t.year(), u8::from(t.month()), t.day(), t.hour(), t.minute())
}

impl FeedbackUi {
    /// Every frame: the toast of a crash this run has not shown yet, and the end of a send.
    fn watch(&mut self, cx: &mut DocCx<'_>) {
        let Some(svc) = FeedbackService::of(cx.editor) else { return };
        if let Some(offer) = svc.offer()
            && self.toasted.as_deref() != Some(offer.crash.name.as_str())
        {
            self.toasted = Some(offer.crash.name.clone());
            gz_ui::state::toast_with(
                cx.ctx,
                Tone::Warning,
                "Grizzly Editor stopped unexpectedly last time.",
                "Send Report...",
                ACTION,
            );
        }
        let answer = self.pending.as_ref().and_then(|slot| slot.lock().take());
        if let Some(reply) = answer {
            self.pending = None;
            // Stop Sending is the author's own doing: nothing to say about it
            if reply.get("code").and_then(Value::as_str) == Some("stopped") {
                return;
            }
            if reply.get("ok").and_then(Value::as_bool) == Some(true) {
                gz_ui::state::toast(cx.ctx, Tone::Success, "Thank you - your report was sent.");
                self.title.clear();
                self.text.clear();
                self.error = None;
                self.crash = None;
                self.open = false;
            } else {
                let error = reply
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("The report was not sent.")
                    .to_owned();
                // a send whose dialog was hidden says so where it is seen, with the way back to the text
                if !self.open {
                    gz_ui::state::toast_with(
                        cx.ctx,
                        Tone::Error,
                        Tx::ui(error.clone()),
                        "Send Feedback...",
                        ACTION,
                    );
                }
                self.error = Some(error);
            }
        }
    }

    /// Opens the dialog as it was left: the text, and the error of a send that failed while it was
    /// hidden (the toast's way back to it).
    fn open_dialog(&mut self, editor: &Arc<Editor>) {
        self.open = true;
        if !self.diagnostics_set {
            self.diagnostics = self.kind == 0;
        }
        self.crash = FeedbackService::of(editor).and_then(|s| s.offer()).map(|o| Offered {
            name: o.crash.name.clone(),
            time: o.crash.time,
            summary: o.crash.summary(),
        });
        self.with_crash = self.crash.is_some();
    }

    fn send(&mut self, editor: &Arc<Editor>) {
        let mut a = args(&[
            ("kind", json!(if self.kind == 0 { "bug" } else { "suggestion" })),
            ("title", json!(self.title.trim())),
            ("text", json!(self.text.trim())),
        ]);
        let crash = self.crash.as_ref().filter(|_| self.with_crash);
        a.insert("diagnostics".into(), json!(self.diagnostics || crash.is_some()));
        if let Some(c) = crash {
            a.insert("crash".into(), json!(c.name));
        }
        let slot = Arc::new(Mutex::new(None));
        self.pending = Some(slot.clone());
        self.error = None;
        let (ed, origin) = (editor.clone(), editor.ui_origin());
        editor.exec.spawn(async move {
            let reply = ed.call("feedback.send", a, origin).await;
            *slot.lock() = Some(reply);
            ed.exec.wake().ping();
        });
    }

    fn crash_card(&mut self, ui: &mut Ui, editor: &Arc<Editor>) {
        let Some(c) = self.crash.clone() else { return };
        let mut dismiss = false;
        gz_ui::card::card(
            ui,
            "gz-ui-app.feedback.crash",
            tr!("Grizzly Editor stopped unexpectedly on {when}.", when = Tx::raw(local(c.time))),
            Tone::Warning,
            true,
            |ui| {
                let p = palette(ui.ctx());
                // the crash's own words: a message, an exception and its module
                text::label(ui, Tx::raw(c.summary.clone()), Font::Body, p.text_secondary, Clip::End);
                ui.add_space(4.0);
                Check::new(&mut self.with_crash, "Send the Crash with This Report")
                    .tip("Its report, the crashed run's log and its dump go with the text, without any name of you or this computer.")
                    .show(ui);
                ui.add_space(2.0);
                dismiss = Button::new("Don't Offer Again")
                    .kind(Kind::Secondary)
                    .tip("The crash stays in the log folder, unsent, and is offered no more.")
                    .show(ui)
                    .clicked();
            },
        );
        if dismiss {
            editor.post("feedback.dismiss", args(&[("crash", json!(c.name))]));
            self.crash = None;
            self.with_crash = false;
        }
    }

    fn draw(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        if !self.open {
            return;
        }
        let editor = cx.editor.clone();
        let available = gz_feedback::available(&editor);
        let sending = self.pending.is_some();
        let stage = editor.jobs.running_in("feedback").map(|j| j.stage).unwrap_or_default();
        let title_len = self.title.trim().chars().count();
        let text_len = self.text.trim().chars().count();
        let why_not: Tx<'static> = if !available {
            "This build carries no key of the report hub, so it cannot send feedback.".into()
        } else if title_len < 5 {
            "Give it a title of 5 characters or more.".into()
        } else if text_len < 20 {
            "Write 20 characters or more: what happened and how to get there again.".into()
        } else if title_len > 200 {
            "A title is 200 characters at most.".into()
        } else if text_len > 10_000 {
            "The text is 10000 characters at most.".into()
        } else {
            Tx::EMPTY
        };
        let ready = why_not.is_empty();
        let mut open = self.open;
        let mut close = false;
        let mut send = false;
        let mut stop = false;
        let error = self.error.clone();
        Dialog::new(Tx::ctx("dlg.feedback", "Send Feedback"), "dlg.feedback", DialogSize::M).show(
            ui,
            &mut open,
            |ui| {
                let p = palette(ui.ctx());
                text::prose(ui, "Tell the editor's author what went wrong or what you would like it to do.");
                ui.add_space(6.0);
                self.crash_card(ui, &editor);
                let before = self.kind;
                gz_ui::segmented(ui, &mut self.kind, &KINDS, None);
                if self.kind != before && !self.diagnostics_set {
                    self.diagnostics = self.kind == 0;
                }
                ui.add_space(6.0);
                gz_ui::form::row_field(ui, "gz-ui-app.feedback.title", &mut self.title, "In a line: what is it about?");
                ui.add_space(4.0);
                let hint = if self.kind == 0 {
                    "What happened, what you expected, and how to get there again."
                } else {
                    "What you would like the editor to do, and what for."
                };
                gz_ui::form::area_field(ui, Id::unique("gz-ui-app.feedback.text"), &mut self.text, "Description", hint, 6);
                ui.add_space(4.0);
                let crash_goes = self.with_crash && self.crash.is_some();
                let mut ticked = self.diagnostics || crash_goes;
                let changed = Check::new(&mut ticked, "Attach Diagnostics")
                    .enabled(!crash_goes, "A crash goes with its diagnostics.")
                    .tip("The editor's state, the end of its log, its settings and the system's description - with every name of you and this computer taken out.")
                    .show(ui)
                    .changed();
                if changed {
                    self.diagnostics = ticked;
                    self.diagnostics_set = true;
                }
                gz_ui::Section::new("gz-ui-app.feedback.more", "More").default_open(false).body(ui, |ui| {
                    text::label(ui, "What Is Sent", Font::Strong, p.text, Clip::End);
                    for line in [
                        "The editor's version and build, Windows, the processor, the memory and the graphics card.",
                        "The editor's state: the open tabs, the jobs, the linked game's version.",
                        "The last 4 MB of the editor's log and its settings.",
                        "For a crash: its report, the crashed run's log and its dump.",
                    ] {
                        text::prose(ui, tr!("\u{2022}  {line}", line = Tx::from(line)));
                    }
                    ui.add_space(4.0);
                    text::prose(ui, "User and computer names, folders, e-mail and network addresses and keys are taken out before anything leaves. What you write goes as you wrote it.");
                });
                if let Some(e) = &error {
                    ui.add_space(4.0);
                    // the command's sentence, the editor's own English
                    text::status(ui, Tone::Error, Tx::ui(e.clone()));
                }
            },
            |f| {
                if sending {
                    f.busy(if stage.is_empty() { Tx::from("Sending the report") } else { Tx::ui(&stage) });
                    stop = f.button("Stop Sending", Kind::Secondary, false, true, "");
                    close = f.button("Hide", Kind::Secondary, false, true, "");
                } else {
                    send = f.button("Send", Kind::Primary, true, ready, why_not.clone());
                    close = f.button("Cancel", Kind::Secondary, false, true, "");
                }
            },
        );
        if send {
            self.send(&editor);
        }
        if stop && let Some(job) = editor.jobs.running_in("feedback") {
            let _ = editor.jobs.cancel(job.id);
        }
        self.open = open && !close;
    }
}

/// Plugs the dialog, Help > Send Feedback... and the crash toast into the shell.
pub(crate) fn install(b: &mut ShellBuilder) {
    let hub = Rc::new(RefCell::new(FeedbackUi::default()));
    let watch = hub.clone();
    // draws nothing: the toast of a crash and the end of a send are looked at every frame
    b.status_chip(6, move |_ui, cx| watch.borrow_mut().watch(cx));
    let (open_hub, draw_hub) = (hub.clone(), hub);
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
    b.action(
        Action::new(ACTION, "Send Feedback...", "Help", "ui.dialog")
            .icon(Icon::Feedback)
            .args(|_| args(&[("action", json!("open")), ("id", json!(DIALOG_ID))])),
    );
    b.menu_action("Help", 20, 1, ACTION, None);
}
