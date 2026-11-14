//! The graph tab's dialogs: Find in Project (`dlg.nq_project_find`), Rename Task
//! (`dlg.nq_task_rename`), Task References (`dlg.nq_task_references`) and Edit Text / Edit Lua
//! (`dlg.nq_text`). What they ask runs off the UI thread (the graph area's project search and
//! references, `graph.edit`'s `rename_task`); the answer lands on a later frame.

use std::cell::Cell;
use std::sync::Arc;

use gz_editor::{Args, Editor, Value as Json};
use gz_ui::{Clip, Dialog, DialogSize, Font, Kind, Tone, palette};
use parking_lot::Mutex;

use crate::params::TextEdit;

fn ok(reply: &Json) -> bool {
    reply.get("ok").and_then(Json::as_bool) == Some(true)
}

fn error_text(reply: &Json) -> String {
    reply.get("error").and_then(Json::as_str).unwrap_or("The command failed.").to_owned()
}

/// Runs a command as the window would and keeps its reply for a later frame.
fn call(
    editor: &Arc<Editor>,
    ctx: &egui::Context,
    name: &'static str,
    args: Args,
    slot: Arc<Mutex<Option<Json>>>,
) {
    let ed = editor.clone();
    let ctx = ctx.clone();
    let origin = editor.ui_origin();
    let _task = editor.exec.spawn(async move {
        let reply = ed.call(name, args, origin).await;
        *slot.lock() = Some(reply);
        ctx.request_repaint();
    });
}

/// What a dialog asks of the graph area, run off the UI thread: its answer lands in `slot`.
fn ask<F, Fut>(editor: &Arc<Editor>, ctx: &egui::Context, f: F, args: Args, slot: Arc<Mutex<Option<Json>>>)
where
    F: FnOnce(Arc<Editor>, Args) -> Fut + Send + 'static,
    Fut: std::future::Future<Output = gz_editor::Reply> + Send + 'static,
{
    let ed = editor.clone();
    let ctx = ctx.clone();
    let _task = editor.exec.spawn(async move {
        let reply = match f(ed, args).await {
            Ok(v) => gz_editor::ok_reply(v),
            Err(e) => gz_editor::error_reply(&e),
        };
        *slot.lock() = Some(reply);
        ctx.request_repaint();
    });
}

fn args(pairs: &[(&str, Json)]) -> Args {
    let mut a = Args::new();
    for (k, v) in pairs {
        a.insert((*k).to_owned(), v.clone());
    }
    a
}

/// What a project search found, as rows.
#[derive(Clone, Debug, Default)]
pub struct FoundRow {
    pub path: String,
    pub node: String,
    pub title: String,
    pub source: String,
}

/// Find in Project: every graph of the project searched without opening one.
#[derive(Default)]
pub struct ProjectFind {
    pub open: bool,
    pub focus: bool,
    pub query: String,
    pub results: Vec<FoundRow>,
    pub diagnostics: Vec<String>,
    pub error: String,
    pub complete: bool,
    pub status: String,
    busy: bool,
    reply: Arc<Mutex<Option<Json>>>,
}

impl ProjectFind {
    pub fn open_with(&mut self, editor: &Arc<Editor>, ctx: &egui::Context, query: &str) {
        if self.query.is_empty() {
            query.clone_into(&mut self.query);
        }
        self.open = true;
        self.focus = true;
        self.search(editor, ctx);
    }

    fn search(&mut self, editor: &Arc<Editor>, ctx: &egui::Context) {
        self.busy = true;
        ask(
            editor,
            ctx,
            gz_nq::project_find,
            args(&[("query", Json::from(self.query.clone()))]),
            self.reply.clone(),
        );
    }

    fn take_reply(&mut self) {
        let Some(r) = self.reply.lock().take() else { return };
        self.busy = false;
        self.results.clear();
        self.diagnostics.clear();
        self.error.clear();
        if !ok(&r) {
            self.error = error_text(&r);
            self.complete = true;
            return;
        }
        self.complete = r.get("complete").and_then(Json::as_bool).unwrap_or(true);
        for x in r.get("results").and_then(Json::as_array).into_iter().flatten() {
            let s = |k: &str| x.get(k).and_then(Json::as_str).unwrap_or("").to_owned();
            self.results.push(FoundRow {
                path: s("path"),
                node: s("node"),
                title: s("title"),
                source: s("source"),
            });
        }
        for d in r.get("diagnostics").and_then(Json::as_array).into_iter().flatten() {
            let s = |k: &str| d.get(k).and_then(Json::as_str).unwrap_or("").to_owned();
            self.diagnostics.push(format!("{}: {}", s("path"), s("message")));
        }
        let generation = r.get("generation").and_then(Json::as_u64).unwrap_or(0);
        let fingerprint = r.get("fingerprint").and_then(Json::as_str).unwrap_or("");
        self.status = if generation > 0 {
            format!(
                "{} \u{b7} index generation {generation} \u{b7} fingerprint {fingerprint}",
                crate::look::plural(self.results.len(), "result", "results")
            )
        } else {
            String::new()
        };
    }

    /// Draws the dialog; the result the author picked: (path, node).
    pub fn draw(&mut self, ui: &mut egui::Ui, editor: &Arc<Editor>) -> Option<(String, String)> {
        self.take_reply();
        if !self.open {
            return None;
        }
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let mut go: Option<usize> = None;
        let mut search_now = false;
        let close = Cell::new(false);
        let status = self.status.clone();
        let busy = self.busy;
        let mut open = self.open;
        Dialog::new("Find in Project", "nq_project_find", DialogSize::L).show(
            ui,
            &mut open,
            |ui| {
                ui.horizontal(|ui| {
                    let w = (ui.available_width() - 104.0).max(gz_ui::units::em(8.0));
                    let r = gz_ui::SearchField::new(&mut self.query)
                        .hint("Find nodes in every graph of the project")
                        .min_em(12.0)
                        .width(w)
                        .show(ui);
                    if self.focus {
                        r.request_focus();
                        self.focus = false;
                    }
                    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        search_now = true;
                    }
                    if gz_ui::Button::new("Search").show(ui).clicked() {
                        search_now = true;
                    }
                });
                if !self.complete {
                    gz_ui::text::status(
                        ui,
                        Tone::Warning,
                        "Incomplete: one or more graph files could not be read.",
                    );
                }
                if !self.error.is_empty() {
                    gz_ui::text::status(ui, Tone::Error, &self.error);
                }
                if busy {
                    gz_ui::state::busy_line(ui, "Searching the project");
                } else if self.results.is_empty() {
                    if self.error.is_empty() {
                        gz_ui::text::status(
                            ui,
                            Tone::Muted,
                            if self.query.is_empty() {
                                "Type what to find, then Search."
                            } else {
                                "No node matches."
                            },
                        );
                    }
                } else {
                    gz_ui::list::list(
                        ui,
                        "gz-ui-graph.project_find",
                        egui::vec2(0.0, 14.0 * 22.0 + 10.0),
                        |l| {
                            for (i, r) in self.results.iter().enumerate() {
                                let text_ = format!(
                                    "{}  \u{b7}  {}  \u{b7}  {}  \u{b7}  {}",
                                    r.path, r.node, r.title, r.source
                                );
                                if l.row(&format!("r{i}"), false, |ui| {
                                    gz_ui::text::label(ui, &text_, Font::Body, p.text, Clip::Middle);
                                })
                                .clicked()
                                {
                                    go = Some(i);
                                }
                            }
                        },
                    );
                }
                for d in &self.diagnostics {
                    gz_ui::text::status(ui, Tone::Warning, d);
                }
            },
            |f| {
                f.status(&status, Tone::Muted);
                if f.button("Close", Kind::Secondary, true, true, "") {
                    close.set(true);
                }
            },
        );
        self.open = open && !close.get();
        if search_now {
            self.search(editor, &ctx);
        }
        let picked = go.and_then(|i| self.results.get(i)).map(|r| (r.path.clone(), r.node.clone()));
        if picked.is_some() {
            self.open = false;
        }
        picked
    }
}

/// Rename Task: changes the task's runtime and save identity.
#[derive(Default)]
pub struct TaskRename {
    pub open: bool,
    pub from: String,
    pub to: String,
    pub error: String,
    busy: bool,
    reply: Arc<Mutex<Option<Json>>>,
}

impl TaskRename {
    pub fn begin(&mut self, id: &str) {
        id.clone_into(&mut self.from);
        id.clone_into(&mut self.to);
        self.error.clear();
        self.open = true;
    }

    pub fn draw(&mut self, ui: &mut egui::Ui, editor: &Arc<Editor>, path: &str) {
        if let Some(r) = self.reply.lock().take() {
            self.busy = false;
            if ok(&r) {
                self.open = false;
                self.from.clear();
            } else {
                self.error = error_text(&r);
            }
        }
        if !self.open {
            return;
        }
        let ctx = ui.ctx().clone();
        let to = self.to.trim().to_owned();
        let can = !to.is_empty() && to != self.from && !self.busy;
        let rename = Cell::new(false);
        let cancel = Cell::new(false);
        let mut open = self.open;
        Dialog::new("Rename Task", "nq_task_rename", DialogSize::S).show(
            ui,
            &mut open,
            |ui| {
                gz_ui::text::prose(ui, &format!("Rename the task '{}'.", self.from));
                ui.add_space(4.0);
                gz_ui::text::label(ui, "New ID", Font::Body, palette(ui.ctx()).text_secondary, Clip::End);
                gz_ui::grid::text_field(ui, &mut self.to);
                ui.add_space(4.0);
                gz_ui::text::status(ui, Tone::Warning, "This changes the task's runtime and save identity.");
                gz_ui::text::status(
                    ui,
                    Tone::Muted,
                    &format!(
                        "Saves and running quest state recorded under '{}' are not migrated.",
                        self.from
                    ),
                );
                if !self.error.is_empty() {
                    gz_ui::text::status(ui, Tone::Error, &self.error);
                }
            },
            |f| {
                if f.button("Rename", Kind::Primary, false, can, "Type the new ID first") {
                    rename.set(true);
                }
                // Cancel is the safe default: the rename reaches saves
                if f.button("Cancel", Kind::Secondary, true, true, "") {
                    cancel.set(true);
                }
            },
        );
        self.open = open && !cancel.get();
        if rename.get() {
            self.busy = true;
            let op = serde_json::json!([{
                "op": "rename_task",
                "from": self.from.clone(),
                "to": to,
                "ack_runtime_identity": true,
            }]);
            let a = args(&[("doc", Json::from(path)), ("ops", op)]);
            call(editor, &ctx, "graph.edit", a, self.reply.clone());
        }
    }
}

/// One reference to a task.
#[derive(Clone, Debug, Default)]
pub struct RefRow {
    pub node: String,
    pub slot: String,
    pub kind: String,
    pub param: String,
}

/// Task References: where the project names a task.
#[derive(Default)]
pub struct TaskRefs {
    pub open: bool,
    pub id: String,
    pub refs: Vec<RefRow>,
    pub diagnostics: Vec<String>,
    pub error: String,
    pub complete: bool,
    pub generation: u64,
    busy: bool,
    reply: Arc<Mutex<Option<Json>>>,
}

impl TaskRefs {
    pub fn begin(&mut self, editor: &Arc<Editor>, ctx: &egui::Context, id: &str, path: &str) {
        id.clone_into(&mut self.id);
        self.refs.clear();
        self.diagnostics.clear();
        self.error.clear();
        self.complete = false;
        self.generation = 0;
        self.open = true;
        self.busy = true;
        let a =
            args(&[("type", Json::from("task_id")), ("value", Json::from(id)), ("path", Json::from(path))]);
        ask(editor, ctx, gz_nq::references, a, self.reply.clone());
    }

    /// Draws the dialog; the reference picked: (node, slot of an action or empty).
    pub fn draw(&mut self, ui: &mut egui::Ui) -> Option<(String, String)> {
        if let Some(r) = self.reply.lock().take() {
            self.busy = false;
            if ok(&r) {
                self.complete = r.get("complete").and_then(Json::as_bool).unwrap_or(true);
                self.generation = r.get("generation").and_then(Json::as_u64).unwrap_or(0);
                for x in r.get("references").and_then(Json::as_array).into_iter().flatten() {
                    let s = |k: &str| x.get(k).and_then(Json::as_str).unwrap_or("").to_owned();
                    self.refs.push(RefRow {
                        node: s("node"),
                        slot: s("slot"),
                        kind: s("kind"),
                        param: s("param"),
                    });
                }
                for d in r.get("diagnostics").and_then(Json::as_array).into_iter().flatten() {
                    self.diagnostics.push(d.get("message").and_then(Json::as_str).unwrap_or("").to_owned());
                }
            } else {
                self.error = error_text(&r);
                self.complete = true;
            }
        }
        if !self.open {
            return None;
        }
        let p = palette(ui.ctx());
        let mut go = None;
        let close = Cell::new(false);
        let status =
            if self.generation > 0 { format!("Index generation {}", self.generation) } else { String::new() };
        let mut open = self.open;
        Dialog::new("Task References", "nq_task_references", DialogSize::M).show(
            ui,
            &mut open,
            |ui| {
                gz_ui::text::prose(
                    ui,
                    &format!("{} reference(s) to the task '{}'", self.refs.len(), self.id),
                );
                if self.busy {
                    gz_ui::state::busy_line(ui, "Looking through the project");
                    return;
                }
                if !self.complete {
                    gz_ui::text::status(
                        ui,
                        Tone::Warning,
                        "Incomplete: some content could not be proven safe.",
                    );
                }
                if !self.error.is_empty() {
                    gz_ui::text::status(ui, Tone::Error, &self.error);
                }
                if self.refs.is_empty() {
                    if self.error.is_empty() {
                        gz_ui::text::status(ui, Tone::Muted, "No typed references.");
                    }
                } else {
                    gz_ui::list::list(
                        ui,
                        "gz-ui-graph.task_refs",
                        egui::vec2(0.0, 12.0 * 22.0 + 10.0),
                        |l| {
                            for (i, r) in self.refs.iter().enumerate() {
                                let text_ =
                                    format!("{}  \u{b7}  {}  \u{b7}  {}.{}", r.node, r.slot, r.kind, r.param);
                                if l.row(&format!("ref{i}"), false, |ui| {
                                    gz_ui::text::label(ui, &text_, Font::Body, p.text, Clip::End);
                                })
                                .clicked()
                                {
                                    go = Some(i);
                                }
                            }
                        },
                    );
                }
                for d in &self.diagnostics {
                    gz_ui::text::status(ui, Tone::Warning, d);
                }
            },
            |f| {
                f.status(&status, Tone::Muted);
                if f.button("Close", Kind::Secondary, true, true, "") {
                    close.set(true);
                }
            },
        );
        self.open = open && !close.get();
        let picked = go.and_then(|i| self.refs.get(i)).map(|r| {
            let base = r.slot.split('/').next().unwrap_or("").to_owned();
            let slot =
                if base.starts_with("enter:") || base.starts_with("exit:") { base } else { String::new() };
            (r.node.clone(), slot)
        });
        if picked.is_some() {
            self.open = false;
        }
        picked
    }
}

/// Edit Text / Edit Lua: a text of more than one line edited in a dialog; Apply leaves it for the
/// field that asked (`DrawTextEdit`).
pub fn draw_text_edit(ui: &mut egui::Ui, te: &mut TextEdit) {
    if !te.open {
        return;
    }
    let apply = Cell::new(false);
    let check = Cell::new(false);
    let cancel = Cell::new(false);
    let status = te.check.clone();
    let ok_check = status == "ok";
    let code = te.code;
    let mut open = te.open;
    let title = if te.code { "Edit Lua" } else { "Edit Text" };
    Dialog::new(title, "nq_text", DialogSize::M).show(
        ui,
        &mut open,
        |ui| {
            gz_ui::text::label(ui, &te.title, Font::Small, palette(ui.ctx()).text_secondary, Clip::End);
            let font = if te.code { Font::Mono.id() } else { Font::Body.id() };
            let h = if te.code { 340.0 } else { 220.0 };
            let w = ui.available_width();
            ui.add_sized(
                egui::vec2(w, h),
                egui::TextEdit::multiline(&mut te.text).font(font).code_editor_if(te.code),
            );
        },
        |f| {
            if status.is_empty() {
            } else if ok_check {
                f.status("Syntax is fine", Tone::Success);
            } else {
                f.status(&status, Tone::Error);
            }
            if f.button("Apply", Kind::Primary, true, true, "") {
                apply.set(true);
            }
            if code && f.button("Check Syntax", Kind::Secondary, false, true, "") {
                check.set(true);
            }
            if f.button("Cancel", Kind::Secondary, false, true, "") {
                cancel.set(true);
            }
        },
    );
    te.open = open && !cancel.get();
    if apply.get() {
        te.apply = true;
        te.open = false;
    }
    if check.get() {
        te.check = match gz_luadata::syntax_check(&te.text) {
            Ok(()) => "ok".to_owned(),
            Err(e) => format!("{} ({}:{})", e.message, e.line, e.column),
        };
    }
}

trait CodeEditorIf {
    fn code_editor_if(self, code: bool) -> Self;
}

impl CodeEditorIf for egui::TextEdit<'_> {
    fn code_editor_if(self, code: bool) -> Self {
        if code { self.code_editor() } else { self }
    }
}
