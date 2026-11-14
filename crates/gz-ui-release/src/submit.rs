//! Submit to Mod Browser (L, `Submit to Mod Browser###dlg.catalog_submit`): a dialog of a FIXED
//! height - what the check finds scrolls inside it, and a running job never resizes it. Cards:
//! Author Key; Requirements (failures first, the met ones folded under "Met (n)", the VirusTotal
//! findings left out); "VirusTotal (required)" - the one place the verdict appears, its open
//! state in the Required tone, never grey; Submission; Withdrawal, whose Withdraw... raises the
//! sheet `dlg.catalog_withdraw`. Every card button is Secondary.
//!
//! A submission is public from its first second, and the catalog judges it by itself: no text
//! here says anyone reviews or merges it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::Ui;
use gz_catalog::model::{vt_level, vt_settled};
use gz_catalog::{Catalog, Job, JobState, View};
use gz_editor::Editor;
use gz_github::scan::{ScanResult, describe};
use gz_github::{AccountSource, CatalogIssue, Github, Secret, vt};
use gz_mod::ManifestService;
use gz_release::content::Level;
use gz_release::{AuthorKeys, KeyState};
use gz_ui::card::card;
use gz_ui::file_dialog::FileRequest;
use gz_ui::grid::read_only;
use gz_ui::text::{self, Clip};
use gz_ui::{Button, Dialog, DialogSize, Font, Icon, Kind, PropFlags, PropertyGrid, Tone, palette};
use parking_lot::Mutex;
use zeroize::Zeroizing;

use gz_ui_mod::KeyLine;

use crate::local::{local_card_title, local_fold, local_secret_field, local_secret_row};
use crate::{open_page, reason};

/// The body's height while the work area has room for it: the room a finished check takes, so a
/// check that ends later scrolls inside instead of growing the window.
const BODY_PT: f32 = 600.0;
const BUSY_WHY: &str = "A Mod Browser job is running.";
const SIGN_IN_WHY: &str = "Sign in to GitHub first: Publish Release.";
const STOP_LABELS: [&str; 5] =
    ["Stop Checking", "Stop Scanning", "Stop Submitting", "Stop Refreshing", "Stop Withdrawing"];
const SUBMIT_LABELS: [&str; 4] = ["Scan and Submit", "Scan and Update", "Update Submission", "Submit"];
const REPORT_LABELS: [&str; 2] = ["Open Report", "Open Reports"];

// ---- the pure part ----------------------------------------------------------------------------

/// Required is never grey: a requirement the submission still has to meet reads in its own tone.
#[must_use]
pub(crate) fn level_tone(level: Level) -> Tone {
    match level {
        Level::Ok => Tone::Success,
        Level::Review => Tone::Warning,
        Level::Required => Tone::Required,
        Level::Block => Tone::Error,
        Level::Note => Tone::Muted,
    }
}

/// VirusTotal has a report page of a package only once it finished analysing it.
#[must_use]
pub(crate) fn has_report(r: &ScanResult) -> bool {
    matches!(r.state.as_str(), "clean" | "warn" | "block")
}

/// The tone of a card about an issue of the catalog: open, accepted, refused, or merely closed.
#[must_use]
pub(crate) fn issue_tone(issue: &CatalogIssue) -> Tone {
    if issue.number == 0 {
        Tone::Muted
    } else if issue.state == "open" {
        Tone::Info
    } else if issue.verdict == "accepted" {
        Tone::Success
    } else if issue.verdict == "refused" {
        Tone::Error
    } else {
        Tone::Muted
    }
}

/// What an issue of the catalog says now, as lines with their tones. Open, the catalog's newest
/// verdict says what it waits for; closed, only an accepted or a refused one says what became of
/// it: the author may have closed it after a waiting one.
#[must_use]
pub(crate) fn issue_lines(
    issue: &CatalogIssue,
    done: bool,
    accepted_done: &str,
    accepted_soon: &str,
) -> Vec<(Tone, String)> {
    let n = issue.number;
    let mut out = Vec::new();
    if issue.state == "open" {
        out.push((Tone::Muted, format!("#{n} is open: the catalog checks it.")));
        let tone = match issue.verdict.as_str() {
            "accepted" => Tone::Success,
            "refused" => Tone::Error,
            _ => Tone::Warning,
        };
        if !issue.verdict_text.is_empty() {
            out.push((tone, issue.verdict_text.clone()));
        }
    } else if issue.verdict == "accepted" {
        out.push((Tone::Success, format!("#{n} {}", if done { accepted_done } else { accepted_soon })));
    } else if issue.verdict == "refused" {
        out.push((Tone::Error, format!("#{n} was refused.")));
        if !issue.verdict_text.is_empty() {
            out.push((Tone::Error, issue.verdict_text.clone()));
        }
    } else {
        out.push((Tone::Muted, format!("#{n} was closed.")));
    }
    out
}

/// The submit button's label: the scan is part of the submission, and the button says so while a
/// report is missing.
#[must_use]
pub(crate) fn submit_label(view: &View) -> &'static str {
    let update = view.submission.number != 0 && view.submission.state == "open";
    let scan = !vt_settled(&view.virustotal);
    match (update, scan) {
        (true, true) => SUBMIT_LABELS[1],
        (true, false) => SUBMIT_LABELS[2],
        (false, true) => SUBMIT_LABELS[0],
        (false, false) => SUBMIT_LABELS[3],
    }
}

/// The Stop button, named after the job.
#[must_use]
pub(crate) fn stop_label(job: Job) -> &'static str {
    match job {
        Job::Vt => STOP_LABELS[1],
        Job::Submit => STOP_LABELS[2],
        Job::Submission => STOP_LABELS[3],
        Job::Withdraw => STOP_LABELS[4],
        Job::Check | Job::None => STOP_LABELS[0],
    }
}

/// The owner of `owner/repo`.
fn owner(repo: &str) -> &str {
    repo.split('/').next().unwrap_or("")
}

// ---- what the pools leave for the frames ------------------------------------------------------

/// A file the system's dialog picked.
enum Picked {
    /// Where a new key goes.
    Target(String),
    /// A key file to sign with from now on.
    Use(String),
}

#[derive(Default)]
struct Facts {
    /// `None` while it is read.
    key: Option<KeyState>,
    key_error: String,
    /// This session made the key: the note to keep a copy stands out.
    key_created: bool,
    /// A VirusTotal key is there; `None` while it is read.
    vt_known: Option<bool>,
    vt_error: String,
    /// Where the GitHub account comes from; `None` while it is read.
    account: Option<AccountSource>,
    catalog: String,
    web: String,
    /// Why a job the window asked for did not start.
    start_error: String,
    picking: bool,
    picked: Option<Picked>,
}

/// What a click of the window asks for, done once it is drawn.
enum Act {
    Check,
    Scan,
    Submit,
    Withdraw,
    Stop,
    CreateKey,
    PickTarget,
    PickKeyFile,
    ShowKeyFile,
    SaveVtKey,
    ForgetVtKey,
    OpenReports,
    AskWithdraw,
    Page(String),
}

/// A sentence in a tone's colour, wrapped at the card's width (prose wraps at 36 em: a card of
/// an L dialog is wider, and the rest of its line would stay empty).
fn wide_status(ui: &mut Ui, tone: Tone, text_: &str) -> egui::Response {
    let color = palette(ui.ctx()).tone(tone);
    ui.add(egui::Label::new(egui::RichText::new(text_).font(Font::Body.id()).color(color)).wrap())
}

pub(crate) struct SubmitUi {
    open: bool,
    was_open: bool,
    /// What `Catalog::window_open` said last frame: MCP's `show` and `hide` move it.
    last_window: bool,
    facts: Arc<Mutex<Facts>>,
    key_target: String,
    key_created: bool,
    /// The password row of a locked key (`gz_ui_mod::KeyLine`).
    key_line: KeyLine,
    /// The row showed the key locked: once it no longer does, the key is read again.
    key_line_locked: bool,
    password: Zeroizing<String>,
    repeat: Zeroizing<String>,
    vt_input: Zeroizing<String>,
    want_withdraw: bool,
    stopping: bool,
    last_state: JobState,
}

impl Default for SubmitUi {
    fn default() -> Self {
        Self {
            open: false,
            was_open: false,
            last_window: false,
            facts: Arc::new(Mutex::new(Facts::default())),
            key_target: String::new(),
            key_created: false,
            key_line: KeyLine::default(),
            key_line_locked: false,
            password: Zeroizing::new(String::with_capacity(128)),
            repeat: Zeroizing::new(String::with_capacity(128)),
            vt_input: Zeroizing::new(String::with_capacity(160)),
            want_withdraw: false,
            stopping: false,
            last_state: JobState::Idle,
        }
    }
}

/// Reads the key, whether a VirusTotal key is there and where the account comes from, off the UI
/// thread. The secrets themselves go at once.
fn refresh_facts(editor: &Arc<Editor>, facts: &Arc<Mutex<Facts>>, account_again: bool) {
    let (ed, facts) = (editor.clone(), facts.clone());
    editor.exec.spawn(async move {
        let key = gz_release::key_state(&ed).await;
        let gh = Github::of(&ed);
        let vt_known = gh.read_vt_key().await.is_some();
        let account = Catalog::of(&ed).account_source(&ed, account_again).await;
        let mut f = facts.lock();
        f.key = Some(key);
        f.vt_known = Some(vt_known);
        f.account = Some(account);
        f.catalog.clone_from(&gh.endpoints().catalog_repo);
        f.web.clone_from(&gh.endpoints().web);
        drop(f);
        ed.exec.wake().ping();
    });
}

/// Starts a job the one way MCP does; why it did not start lands in `slot`.
fn start_job(editor: &Arc<Editor>, facts: &Arc<Mutex<Facts>>, job: Job) {
    let (ed, facts) = (editor.clone(), facts.clone());
    editor.exec.spawn(async move {
        let started = Catalog::of(&ed).start(&ed, job, true).await;
        let why = started.err().unwrap_or_default();
        let mut f = facts.lock();
        if job == Job::Vt {
            f.vt_error = why
        } else {
            f.start_error = why
        }
        drop(f);
        ed.exec.wake().ping();
    });
}

/// The system's file dialog on a thread of its own; what it picked comes back to a frame.
fn pick_file(editor: &Arc<Editor>, facts: &Arc<Mutex<Facts>>, save: bool, start: &str) {
    {
        let mut f = facts.lock();
        if f.picking {
            return;
        }
        f.picking = true;
    }
    let title = if save { "Where the new author key goes" } else { "Use an author key file" };
    let at = Path::new(start);
    let mut request = match at.file_name().and_then(|n| n.to_str()) {
        Some(name) if save => FileRequest::save(title, name),
        _ => FileRequest::open(title),
    }
    .filter("Grizzly author key (*.xkey)", &["xkey"])
    .filter("All files", &["*"]);
    if let Some(dir) = at.parent().filter(|d| !d.as_os_str().is_empty()) {
        request = request.in_folder(dir);
    }
    let done = PickDone { slot: facts.clone(), editor: editor.clone() };
    let asker = if editor.flags.unattended { gz_ui::Asker::Unattended } else { gz_ui::Asker::Ui };
    let picked = gz_ui::file_dialog::pick(None, asker, request, move |p: PathBuf| {
        let p = p.display().to_string();
        done.slot.lock().picked = Some(if save { Picked::Target(p) } else { Picked::Use(p) });
    });
    if let Err(line) = picked {
        editor.log.finding(&line);
    }
}

/// Ends a key file dialog when its answer is dropped, picked or not: the window takes the pick
/// at its next frame.
struct PickDone {
    slot: Arc<Mutex<Facts>>,
    editor: Arc<Editor>,
}

impl Drop for PickDone {
    fn drop(&mut self) {
        self.slot.lock().picking = false;
        self.editor.exec.wake().ping();
    }
}

impl SubmitUi {
    /// Mod > Submit to Mod Browser, `ui.dialog open id=submit`: the window, and the requirements
    /// as they stand now.
    pub(crate) fn request(&mut self, editor: &Arc<Editor>) {
        if editor.project_opt().is_none() {
            return;
        }
        let catalog = Catalog::of(editor);
        catalog.set_window(true);
        self.last_window = true;
        self.open_view(editor, true);
        let (ed, facts) = (editor.clone(), self.facts.clone());
        editor.exec.spawn(async move {
            let catalog = Catalog::of(&ed);
            if !catalog.running()
                && let Err(e) = catalog.start(&ed, Job::Check, true).await
            {
                facts.lock().start_error = e;
            }
            ed.exec.wake().ping();
        });
    }

    /// The window's fields start over; what it shows is read off the UI thread.
    fn open_view(&mut self, editor: &Arc<Editor>, account_again: bool) {
        self.facts = Arc::new(Mutex::new(Facts::default()));
        self.key_target.clear();
        self.key_created = false;
        self.want_withdraw = false;
        self.stopping = false;
        self.wipe();
        self.key_line = KeyLine::default();
        self.key_line_locked = false;
        self.key_line.refresh(editor);
        refresh_facts(editor, &self.facts, account_again);
        self.open = true;
    }

    fn wipe(&mut self) {
        self.password = Zeroizing::new(String::with_capacity(128));
        self.repeat = Zeroizing::new(String::with_capacity(128));
        self.vt_input = Zeroizing::new(String::with_capacity(160));
    }

    pub(crate) fn draw(&mut self, ui: &mut Ui, cx: &mut gz_app::DocCx<'_>) {
        let editor = cx.editor.clone();
        let catalog = Catalog::of(&editor);
        // MCP's show and hide move the window's flag
        let window = catalog.window_open();
        if window != self.last_window {
            if window && !self.open {
                self.open_view(&editor, false);
            } else if !window && self.open {
                self.open = false;
            }
            self.last_window = window;
        }
        if editor.project_opt().is_none() {
            self.open = false;
        }
        if self.was_open && !self.open {
            self.wipe();
            self.key_line = KeyLine::default();
            self.want_withdraw = false;
            catalog.set_window(false);
            self.last_window = false;
        }
        self.was_open = self.open;
        if !self.open {
            return;
        }
        if catalog.wants_withdraw_question() {
            self.want_withdraw = true;
        }

        let view = catalog.view();
        // a job that ended: what it may have changed is read again
        if self.last_state == JobState::Running && view.state != JobState::Running {
            self.stopping = false;
            refresh_facts(&editor, &self.facts, false);
        }
        self.last_state = view.state;
        self.take_picked(&editor);
        // a password given in the key row: the card reads the key again
        if self.key_line.wants_password() {
            self.key_line_locked = true;
        } else if self.key_line_locked {
            self.key_line_locked = false;
            refresh_facts(&editor, &self.facts, false);
        }
        let (name, id) = manifest_names(&editor);
        let (key, key_error, vt_known, vt_error, account, catalog_repo, web, start_error, picking) = {
            let mut f = self.facts.lock();
            if f.key_created {
                f.key_created = false;
                self.key_created = true;
            }
            (
                f.key.clone(),
                f.key_error.clone(),
                f.vt_known,
                f.vt_error.clone(),
                f.account,
                f.catalog.clone(),
                f.web.clone(),
                f.start_error.clone(),
                f.picking,
            )
        };
        if let Some(k) = &key
            && (self.key_target.is_empty() || k.exists)
            && self.key_target != k.path
        {
            self.key_target.clone_from(&k.path);
        }
        let signed_in = matches!(account, Some(a) if a != AccountSource::None);
        let running = view.state == JobState::Running;

        let mut acts: Vec<Act> = Vec::new();
        let mut open = self.open;
        let had_question = self.want_withdraw;
        let ctx = ui.ctx().clone();

        let blocks: Vec<&str> =
            view.check.findings.iter().filter(|f| f.level == Level::Block).map(|f| f.text.as_str()).collect();
        let ready = view.check.done && !view.check.blocking && signed_in && !running;
        let why = if running {
            BUSY_WHY
        } else if !signed_in {
            SIGN_IN_WHY
        } else if !view.check.done {
            "Check the requirements first."
        } else {
            blocks.first().copied().unwrap_or("Every requirement above has to be met.")
        };
        let status = if !running && view.check.done && !blocks.is_empty() {
            format!(
                "{} {} to fix",
                blocks.len(),
                if blocks.len() == 1 { "requirement" } else { "requirements" }
            )
        } else {
            String::new()
        };
        let (stop_name, submit_name, stopping) = (stop_label(view.job), submit_label(&view), self.stopping);
        let stage = if view.stage.is_empty() { "Working...".to_owned() } else { view.stage.clone() };
        let (mut stop, mut submit, mut close) = (false, false, false);

        Dialog::new("Submit to Mod Browser", "dlg.catalog_submit", DialogSize::L).show(
            ui,
            &mut open,
            |ui| {
                let top = ui.cursor().min.y;
                let p = palette(ui.ctx());
                // the module, and what the window does for it
                ui.horizontal(|ui| {
                    let id_w = text::width(ui.ctx(), &Font::Body.id(), &id);
                    ui.scope(|ui| {
                        ui.set_max_width((ui.available_width() - id_w - gz_ui::units::em(2.0)).max(gz_ui::units::em(6.0)));
                        text::label(ui, if name.is_empty() { &id } else { &name }, Font::Title, p.text, Clip::End);
                    });
                    if !name.is_empty() && name != id {
                        text::label(ui, &id, Font::Body, p.text_muted, Clip::End);
                    }
                });
                wide_status(
                    ui,
                    Tone::Muted,
                    "Lists the module in the Mod Browser of Dead Air: Refined. The catalog judges every submission by itself and lists the ones that pass its checks.",
                );
                ui.add_space(4.0);
                self.key_card(ui, &editor, key.as_ref(), &key_error, picking, &mut acts);
                requirements_card(ui, &view, &start_error, &mut acts);
                self.vt_card(ui, &view, vt_known, &vt_error, &mut acts);
                submission_card(ui, &view, signed_in, &catalog_repo, &mut acts);
                withdrawal_card(ui, &view, signed_in, &mut acts);
                // a fixed height: the body asks for the room a finished check takes
                let used = ui.cursor().min.y - top;
                if used < BODY_PT {
                    ui.add_space(BODY_PT - used);
                }
                self.withdraw_sheet(ui, &view, signed_in, &name, &id, &catalog_repo, &mut acts);
            },
            |f| {
                f.status(&status, Tone::Warning);
                // the stage on the left, in what the buttons leave; the job's Stop is the first button, named after the job
                // while a job runs: its stage and its one Stop; the title's cross hides the window
                if running {
                    f.busy(&stage);
                    stop = f.button_set(stop_name, &STOP_LABELS, Kind::Secondary, false, !stopping, "Stopping at the next step.");
                } else {
                    submit = f.button_set(submit_name, &SUBMIT_LABELS, Kind::Primary, false, ready, why);
                    close = f.button("Close", Kind::Secondary, false, true, "");
                }
            },
        );
        if stop {
            acts.push(Act::Stop);
        }
        if submit {
            acts.push(Act::Submit);
        }
        self.open = open && !close;
        if had_question && !self.want_withdraw {
            catalog.set_withdraw_question(false);
        }
        for act in acts {
            self.run(act, &ctx, &editor, &view, &web);
        }
    }

    /// What the system's file dialog picked, in this frame.
    fn take_picked(&mut self, editor: &Arc<Editor>) {
        let picked = self.facts.lock().picked.take();
        match picked {
            Some(Picked::Target(path)) => self.key_target = path,
            Some(Picked::Use(path)) => {
                self.key_created = false;
                let (ed, facts) = (editor.clone(), self.facts.clone());
                editor.exec.spawn(async move {
                    let keys = AuthorKeys::of(&ed);
                    let project = ed.project_root();
                    let done = ed.exec.io(move || keys.use_file(&path, project.as_deref())).await;
                    let state = gz_release::key_state(&ed).await;
                    let mut f = facts.lock();
                    f.key_error = done.err().unwrap_or_default();
                    f.key = Some(state);
                    drop(f);
                    ed.exec.wake().ping();
                });
            }
            None => {}
        }
    }

    fn key_card(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        key: Option<&KeyState>,
        key_error: &str,
        picking: bool,
        acts: &mut Vec<Act>,
    ) {
        let Some(key) = key else {
            card(ui, "card_key", "Author Key", Tone::Muted, false, |ui| {
                wide_status(ui, Tone::Muted, "Reading the author key...");
            });
            return;
        };
        let readable = key.exists && key.error.is_empty();
        let tone = if !readable {
            Tone::Error
        } else if key.locked {
            Tone::Required
        } else {
            Tone::Success
        };
        card(ui, "card_key", "", tone, !readable || key.locked, |ui| {
            let badge = readable.then_some((if key.locked { "Locked" } else { "Ready" }, tone));
            local_card_title(ui, "Author Key", tone, false, badge, None);
            if !key.exists {
                wide_status(
                    ui,
                    Tone::Muted,
                    "No author key yet. It signs every release of the module, and the Mod Browser lists signed releases only. It is a file you keep - never part of the project.",
                );
                let differ = *self.password != *self.repeat;
                let target = self.key_target.clone();
                PropertyGrid::new("catalog_key").label_frac(gz_ui::units::em(8.0) / ui.available_width().max(1.0)).show(ui, |g| {
                    g.row("Key File", "Where Create Key writes the new key file. It never replaces a file that is there.", PropFlags::NONE, 0, |ui| {
                        read_only(ui, &target, Clip::Start);
                    });
                    if !picking && g.trailing(Icon::Open, "Choose where the new key goes") {
                        acts.push(Act::PickTarget);
                    }
                    let help = "Optional. Protects the file if it ever leaves your hands; the editor asks for it once per session. Forgotten, it cannot be recovered - and neither can the key.";
                    g.row("Password", help, PropFlags::NONE, 0, |ui| {
                        let w = ui.available_width();
                        local_secret_field(ui, "new_password", &mut self.password, "Optional", w);
                    });
                    let wrong = differ && !self.repeat.is_empty();
                    g.row("Repeat", "The password once more.", if wrong { PropFlags::INVALID } else { PropFlags::NONE }, 0, |ui| {
                        let w = ui.available_width();
                        local_secret_field(ui, "repeat_password", &mut self.repeat, "Repeat the password", w);
                    });
                    if wrong {
                        g.message(Tone::Error, "The passwords differ.");
                    }
                });
                ui.horizontal(|ui| {
                    if Button::new("Create Key")
                        .enabled(!differ, "The two passwords differ.")
                        .show(ui)
                        .clicked()
                    {
                        acts.push(Act::CreateKey);
                    }
                    if Button::new("Use Key File...")
                        .enabled(!picking, "A file dialog is open.")
                        .tip("The key file from another computer, or its copy.")
                        .show(ui)
                        .clicked()
                    {
                        acts.push(Act::PickKeyFile);
                    }
                });
            } else if !key.error.is_empty() {
                wide_status(ui, Tone::Error, &format!("The key file does not read: {}.", key.error));
                PropertyGrid::new("catalog_key")
                    .label_frac(gz_ui::units::em(8.0) / ui.available_width().max(1.0))
                    .show(ui, |g| {
                        g.row(
                            "Key File",
                            "The key file the editor remembers.",
                            PropFlags::READ_ONLY,
                            0,
                            |ui| read_only(ui, &key.path, Clip::Start),
                        );
                    });
                if Button::new("Use Another Key File...")
                    .enabled(!picking, "A file dialog is open.")
                    .show(ui)
                    .clicked()
                {
                    acts.push(Act::PickKeyFile);
                }
            } else if key.locked || self.key_created {
                self.key_details(ui, editor, key, picking, acts);
            } else {
                local_fold(ui, "catalog_key_more", "More", false, |ui| {
                    self.key_details(ui, editor, key, picking, acts)
                });
            }
            if !key_error.is_empty() {
                wide_status(ui, Tone::Error, key_error);
            }
        });
    }

    fn key_details(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        key: &KeyState,
        picking: bool,
        acts: &mut Vec<Act>,
    ) {
        PropertyGrid::new("catalog_key")
            .label_frac(gz_ui::units::em(8.0) / ui.available_width().max(1.0))
            .show(ui, |g| {
                g.row("Key", "The key's id: what a signed release names.", PropFlags::READ_ONLY, 0, |ui| {
                    read_only(ui, &key.key_id, Clip::End)
                });
                g.row("Key File", "The key file this computer signs with.", PropFlags::READ_ONLY, 0, |ui| {
                    read_only(ui, &key.path, Clip::Start)
                });
                if key.locked {
                    // the author key row Package Release and Publish Release draw: the password, once per session
                    let help = "The key file is protected: its password opens it until the editor closes.";
                    g.row("Password", help, PropFlags::REQUIRED, 0, |ui| {
                        self.key_line.draw(ui, editor, "submit")
                    });
                }
            });
        wide_status(
            ui,
            if self.key_created { Tone::Warning } else { Tone::Muted },
            "Keep a copy of this file somewhere safe - a USB stick, a password manager. Without it no update of this module can be signed.",
        );
        // the card's actions, named and framed like every other button of the window
        ui.horizontal(|ui| {
            let use_tip =
                "Signs with another key file from now on: the key file from another computer, or its copy.";
            if Button::new("Use Another Key File...")
                .enabled(!picking, "A file dialog is open.")
                .tip(use_tip)
                .show(ui)
                .clicked()
            {
                acts.push(Act::PickKeyFile);
            }
            if Button::new("Show in Folder")
                .tip(
                    "Opens the key file's folder in Explorer, the file selected - to copy it somewhere safe.",
                )
                .show(ui)
                .clicked()
            {
                acts.push(Act::ShowKeyFile);
            }
        });
    }

    /// The one place the VirusTotal verdict appears; required, so its open state is never grey.
    fn vt_card(
        &mut self,
        ui: &mut Ui,
        view: &View,
        vt_known: Option<bool>,
        vt_error: &str,
        acts: &mut Vec<Act>,
    ) {
        let settled = vt_settled(&view.virustotal);
        let running = view.state == JobState::Running;
        let scanning = running && matches!(view.job, Job::Vt | Job::Submit);
        let packages = &view.virustotal.packages;
        let tone = if vt_known == Some(true) && !packages.is_empty() {
            level_tone(vt_level(&view.virustotal.state, view.vt_scanned))
        } else {
            Tone::Required
        };
        card(ui, "card_vt", "", tone, !settled, |ui| {
            local_card_title(ui, "VirusTotal (required)", tone, !settled, None, None);
            match vt_known {
                None => {
                    wide_status(
                        ui,
                        Tone::Required,
                        "Required: every package of the release needs a VirusTotal report before the module is listed.",
                    );
                }
                Some(false) => {
                    wide_status(
                        ui,
                        Tone::Required,
                        "Required: the catalog lists a package only when VirusTotal knows it, and every release is scanned with your own VirusTotal key. A free account at virustotal.com has one.",
                    );
                    let typed = !self.vt_input.is_empty();
                    if local_secret_row(
                        ui,
                        "vt_key",
                        &mut self.vt_input,
                        "VirusTotal API key",
                        "Save Key",
                        typed,
                        "Paste the key first.",
                    ) {
                        acts.push(Act::SaveVtKey);
                    }
                    wide_status(
                        ui,
                        Tone::Muted,
                        "Kept in the Windows Credential Manager - never in the project.",
                    );
                }
                Some(true) => {
                    // a check looks the packages up: until it has, "not scanned" would be a guess
                    let looking = running && view.job == Job::Check && packages.is_empty();
                    if scanning {
                        wide_status(ui, Tone::Muted, "Scanning...");
                    } else if looking {
                        wide_status(ui, Tone::Muted, "Looking the packages up on VirusTotal...");
                    }
                    // the requirement stays in sight until every package has a report
                    if packages.is_empty() && !looking {
                        wide_status(
                            ui,
                            Tone::Required,
                            "Not scanned yet. Every package needs a VirusTotal report before the module is listed: Scan runs it now, and Submit scans first.",
                        );
                    } else if !packages.is_empty() && !settled {
                        wide_status(
                            ui,
                            Tone::Required,
                            "Not every package has a VirusTotal report yet: the module is not submitted without one.",
                        );
                    }
                    for r in packages {
                        wide_status(
                            ui,
                            level_tone(vt_level(&r.state, view.vt_scanned)),
                            &format!("{}.", describe(r)),
                        );
                    }
                    let reports = packages.iter().filter(|r| has_report(r)).count();
                    ui.horizontal(|ui| {
                        let scan_tip = "Looks every package of the latest release up on VirusTotal, uploads one it does not know and waits for its report - a few minutes. VirusTotal shares uploaded files with the security industry.";
                        if Button::new("Scan").enabled(!running, BUSY_WHY).tip(scan_tip).show(ui).clicked() {
                            acts.push(Act::Scan);
                        }
                        if !packages.is_empty() {
                            let label = if reports > 1 { REPORT_LABELS[1] } else { REPORT_LABELS[0] };
                            let tip = if reports > 1 { "Opens the VirusTotal report of every package in the browser." } else { "Opens the package's VirusTotal report in the browser." };
                            if Button::new(label).label_set(&REPORT_LABELS).enabled(reports > 0, "No package has a finished VirusTotal report yet.").tip(tip).show(ui).clicked() {
                                acts.push(Act::OpenReports);
                            }
                        }
                        let forget_tip = "Removes the VirusTotal API key the editor saved in the Windows Credential Manager. A key from VT_API_KEY stays in effect. The author key is not touched.";
                        if Button::new("Forget VirusTotal Key").tip(forget_tip).show(ui).clicked() {
                            acts.push(Act::ForgetVtKey);
                        }
                    });
                }
            }
            if !vt_error.is_empty() {
                wide_status(ui, Tone::Error, vt_error);
            }
        });
    }

    /// The one question before a withdrawal leaves, as a sheet over the window: what it does, for
    /// whom, and that it is public. Cancel is the default - Enter and Esc both back out.
    #[allow(clippy::too_many_arguments)]
    fn withdraw_sheet(
        &mut self,
        ui: &mut Ui,
        view: &View,
        signed_in: bool,
        name: &str,
        id: &str,
        catalog_repo: &str,
        acts: &mut Vec<Act>,
    ) {
        let running = view.state == JobState::Running;
        let can = !running && signed_in;
        let mut go = false;
        let mut cancel = false;
        let headline =
            format!("Withdraw {} ({id}) from the Mod Browser?", if name.is_empty() { id } else { name });
        let from = if view.login.is_empty() { String::new() } else { format!(", from {}", view.login) };
        let where_ = format!(
            "The request is a public issue at github.com/{catalog_repo}{from}, judged by the catalog's own checks."
        );
        Dialog::new("Withdraw from Mod Browser", "dlg.catalog_withdraw", DialogSize::M).sheet().show(
            ui,
            &mut self.want_withdraw,
            |ui| {
                let p = palette(ui.ctx());
                ui.scope(|ui| {
                    ui.add(egui::Label::new(egui::RichText::new(&headline).font(Font::Strong.id()).color(p.text)).wrap());
                });
                ui.add_space(4.0);
                wide_status(ui, Tone::Warning, "The Mod Browser stops offering it, and players who installed it there get no more updates.");
                wide_status(ui, Tone::Muted, "Installed copies keep working.");
                wide_status(ui, Tone::Muted, "The id stays bound to your author key: submitting the module again lists it again.");
                wide_status(ui, Tone::Muted, &where_);
            },
            |f| {
                go = f.button("Withdraw", Kind::Danger, false, can, if signed_in { BUSY_WHY } else { SIGN_IN_WHY });
                cancel = f.button("Cancel", Kind::Secondary, true, true, "");
            },
        );
        if go {
            acts.push(Act::Withdraw);
        }
        if go || cancel {
            self.want_withdraw = false;
        }
    }

    fn run(&mut self, act: Act, ctx: &egui::Context, editor: &Arc<Editor>, view: &View, web: &str) {
        match act {
            Act::Check => start_job(editor, &self.facts, Job::Check),
            Act::Scan => start_job(editor, &self.facts, Job::Vt),
            Act::Submit => start_job(editor, &self.facts, Job::Submit),
            Act::Withdraw => start_job(editor, &self.facts, Job::Withdraw),
            Act::Stop => {
                Catalog::of(editor).cancel(editor);
                self.stopping = true;
            }
            Act::CreateKey => {
                let password = Zeroizing::new(std::mem::take(&mut *self.password));
                self.wipe();
                let (ed, facts, target) = (editor.clone(), self.facts.clone(), self.key_target.clone());
                editor.exec.spawn(async move {
                    let keys = AuthorKeys::of(&ed);
                    let project = ed.project_root();
                    let done =
                        ed.exec.io(move || keys.create(Some(&target), &password, project.as_deref())).await;
                    let state = gz_release::key_state(&ed).await;
                    let mut f = facts.lock();
                    f.key_created = done.is_ok();
                    f.key_error = done.err().unwrap_or_default();
                    f.key = Some(state);
                    drop(f);
                    ed.exec.wake().ping();
                });
            }
            Act::PickTarget => pick_file(editor, &self.facts, true, &self.key_target),
            Act::PickKeyFile => {
                let start = self.facts.lock().key.as_ref().map(|k| k.path.clone()).unwrap_or_default();
                pick_file(editor, &self.facts, false, &start);
            }
            Act::ShowKeyFile => {
                if let Some(k) = self.facts.lock().key.as_ref() {
                    let mut arg = std::ffi::OsString::from("/select,");
                    arg.push(&k.path);
                    let _ = std::process::Command::new("explorer.exe").arg(arg).spawn();
                }
            }
            Act::SaveVtKey => {
                let key = Zeroizing::new(std::mem::take(&mut *self.vt_input));
                self.vt_input = Zeroizing::new(String::with_capacity(160));
                let (ed, facts) = (editor.clone(), self.facts.clone());
                editor.exec.spawn(async move {
                    let gh = Github::of(&ed);
                    let saved = gh.save_vt_key(Secret::from(key)).await;
                    let known = gh.read_vt_key().await.is_some();
                    let mut f = facts.lock();
                    f.vt_error = saved.err().unwrap_or_default();
                    f.vt_known = Some(known);
                    drop(f);
                    ed.exec.wake().ping();
                });
            }
            Act::ForgetVtKey => {
                let (ed, facts) = (editor.clone(), self.facts.clone());
                editor.exec.spawn(async move {
                    let gh = Github::of(&ed);
                    gh.forget_vt_key().await;
                    let known = gh.read_vt_key().await.is_some();
                    facts.lock().vt_known = Some(known);
                    ed.exec.wake().ping();
                });
            }
            Act::OpenReports => {
                for r in view.virustotal.packages.iter().filter(|r| has_report(r)) {
                    open_page(ctx, &vt::report_page(&r.package.sha256), web);
                }
            }
            Act::AskWithdraw => self.want_withdraw = true,
            Act::Page(url) => open_page(ctx, &url, web),
        }
    }
}

/// The module's title and id, from the manifest store's snapshot.
fn manifest_names(editor: &Editor) -> (String, String) {
    let Some(session) = editor.project_opt() else { return (String::new(), String::new()) };
    let Some(svc) = ManifestService::get(&session) else { return (String::new(), String::new()) };
    let snap = svc.snapshot();
    snap.manifest().map_or((String::new(), String::new()), |m| (m.name.clone(), m.id.clone()))
}

/// What section 7 asks, failures first; the requirements that are met fold away - a wall of green
/// reads as nothing. The VirusTotal findings are the VirusTotal card's.
fn requirements_card(ui: &mut Ui, view: &View, start_error: &str, acts: &mut Vec<Act>) {
    let mut open: Vec<_> =
        view.check.findings.iter().filter(|f| f.code != "virustotal" && f.level != Level::Ok).collect();
    let met: Vec<_> =
        view.check.findings.iter().filter(|f| f.code != "virustotal" && f.level == Level::Ok).collect();
    open.sort_by_key(|a| std::cmp::Reverse(a.level));
    let blocks = open.iter().filter(|f| f.level == Level::Block).count();
    let required = open.iter().filter(|f| f.level == Level::Required).count();
    let running = view.state == JobState::Running;
    let checking = running && matches!(view.job, Job::Check | Job::Submit);
    let tone = if blocks > 0 {
        Tone::Error
    } else if required > 0 {
        Tone::Required
    } else if view.check.done {
        Tone::Success
    } else {
        Tone::Muted
    };
    card(ui, "card_requirements", "", tone, blocks > 0, |ui| {
        let fix = format!("{blocks} to fix");
        let badge = (view.check.done && !checking).then_some(if blocks > 0 {
            (fix.as_str(), Tone::Error)
        } else {
            ("Met", Tone::Success)
        });
        if local_card_title(ui, "Requirements", tone, false, badge, Some(("Check Again", !running, BUSY_WHY)))
        {
            acts.push(Act::Check);
        }
        if checking {
            wide_status(ui, Tone::Muted, "Checking...");
        } else if !view.check.done {
            wide_status(ui, Tone::Muted, "Not checked yet.");
        }
        for f in &open {
            wide_status(ui, level_tone(f.level), &f.text);
        }
        if view.check.done && open.is_empty() && !checking {
            wide_status(ui, Tone::Success, "Every requirement is met.");
        }
        if !met.is_empty() {
            local_fold(ui, "catalog_met", &format!("Met ({})", met.len()), false, |ui| {
                for f in &met {
                    wide_status(ui, Tone::Success, &f.text);
                }
            });
        }
        if !start_error.is_empty() {
            wide_status(ui, Tone::Error, start_error);
        }
    });
}

fn submission_card(ui: &mut Ui, view: &View, signed_in: bool, catalog_repo: &str, acts: &mut Vec<Act>) {
    let submission = &view.submission;
    let tone = issue_tone(submission);
    card(ui, "card_submission", "Submission", tone, tone == Tone::Error, |ui| {
        // "not submitted" only once GitHub said so: every job but the scan looks the submission up
        if !view.submission_read {
            if signed_in && view.state == JobState::Running && view.job != Job::Vt {
                wide_status(ui, Tone::Muted, "Looking up the submission...");
            } else if signed_in && !view.submission_error.is_empty() {
                wide_status(
                    ui,
                    Tone::Warning,
                    &format!("The submission could not be looked up: {}.", reason(&view.submission_error)),
                );
            } else if signed_in {
                wide_status(ui, Tone::Muted, "Not looked up yet: Check Again reads where it stands.");
            }
        } else if submission.number == 0 {
            wide_status(ui, Tone::Muted, "Not submitted yet.");
        } else {
            for (tone, line) in issue_lines(
                submission,
                view.listed(),
                "was accepted: the module is in the Mod Browser.",
                "was accepted: the module appears in the Mod Browser within minutes.",
            ) {
                wide_status(ui, tone, &line);
            }
            if !submission.note.is_empty() {
                wide_status(ui, Tone::Muted, &submission.note);
            }
            if Button::new("Open Submission").show(ui).clicked() {
                acts.push(Act::Page(submission.url.clone()));
            }
        }
        let from = if view.login.is_empty() { String::new() } else { format!(" from {}", view.login) };
        let where_ = format!("Submitting opens a public issue at github.com/{catalog_repo}{from}.");
        wide_status(
            ui,
            Tone::Muted,
            if signed_in {
                &where_
            } else {
                "Submitting needs a GitHub account: sign in from Publish Release."
            },
        );
        if view.state == JobState::Failed && !view.error.is_empty() && view.job != Job::Withdraw {
            wide_status(ui, Tone::Error, &view.error);
        }
    });
}

/// A listed module can be withdrawn by the account it was listed from; a withdrawal already sent
/// is followed like the submission.
fn withdrawal_card(ui: &mut Ui, view: &View, signed_in: bool, acts: &mut Vec<Act>) {
    let w = &view.withdrawal;
    let listed = view.listing.state == "listed";
    let withdrawn = view.listing.state == "withdrawn";
    let failed = view.state == JobState::Failed && view.job == Job::Withdraw && !view.error.is_empty();
    if !listed && !withdrawn && w.number == 0 && w.note.is_empty() && !failed {
        return;
    }
    let tone = if failed { Tone::Error } else { issue_tone(w) };
    let running = view.state == JobState::Running;
    card(ui, "card_withdrawal", "Withdrawal", tone, failed, |ui| {
        if running && view.job == Job::Withdraw {
            wide_status(ui, Tone::Muted, "Withdrawing...");
        }
        if w.number != 0 {
            for (tone, line) in issue_lines(
                w,
                withdrawn,
                "was accepted: the module is withdrawn from the Mod Browser.",
                "was accepted: the module leaves the Mod Browser within minutes.",
            ) {
                wide_status(ui, tone, &line);
            }
        }
        if !w.note.is_empty() {
            wide_status(ui, Tone::Muted, &w.note);
        }
        if withdrawn && w.number != 0 && w.verdict == "accepted" {
            wide_status(ui, Tone::Muted, "Submitting the module again lists it again.");
        } else if withdrawn {
            wide_status(
                ui,
                Tone::Muted,
                "Not in the Mod Browser: its author withdrew it. Submitting lists it again.",
            );
        }
        if listed {
            wide_status(
                ui,
                Tone::Muted,
                "Withdrawing takes the module out of the Mod Browser: it is no longer offered, and players who installed it there get no more updates. Installed copies keep working.",
            );
        }
        // only the account the listing came from may ask; the catalog checks it again by itself
        let owner = owner(&view.listing.github);
        let other_account = listed && !view.login.is_empty() && !owner.eq_ignore_ascii_case(&view.login);
        if other_account {
            wide_status(
                ui,
                Tone::Warning,
                &format!("Only {owner}, the owner of {}, can withdraw it.", view.listing.github),
            );
        }
        ui.horizontal(|ui| {
            if listed {
                let can = !running && signed_in && !other_account;
                let why = if other_account {
                    "Only the account the module was listed from can withdraw it."
                } else if running {
                    BUSY_WHY
                } else {
                    SIGN_IN_WHY
                };
                let tip = "Asks the catalog to withdraw the module: a public issue in the catalog repository, judged by the catalog's own checks. The id stays bound to your author key. Asks first.";
                // the button that opens the question is quiet; the one that acts, inside the question, is the Danger one
                if Button::new("Withdraw...").kind(Kind::GhostDanger).enabled(can, why).tip(tip).show(ui).clicked() {
                    acts.push(Act::AskWithdraw);
                }
            }
            if w.number != 0 && Button::new("Open Withdrawal").show(ui).clicked() {
                acts.push(Act::Page(w.url.clone()));
            }
        });
        if failed {
            wide_status(ui, Tone::Error, &view.error);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn issue(number: u64, state: &str, verdict: &str, text_: &str) -> CatalogIssue {
        CatalogIssue {
            number,
            state: state.to_owned(),
            verdict: verdict.to_owned(),
            verdict_text: text_.to_owned(),
            ..CatalogIssue::default()
        }
    }

    #[test]
    fn required_is_never_grey() {
        assert_eq!(level_tone(Level::Required), Tone::Required);
        assert_eq!(level_tone(vt_level("unknown", false)), Tone::Required);
        assert_eq!(level_tone(vt_level("clean", true)), Tone::Success);
        assert_eq!(level_tone(vt_level("block", true)), Tone::Error);
    }

    #[test]
    fn issue_tones_and_lines() {
        assert_eq!(issue_tone(&CatalogIssue::default()), Tone::Muted);
        assert_eq!(issue_tone(&issue(4, "open", "", "")), Tone::Info);
        assert_eq!(issue_tone(&issue(4, "closed", "accepted", "")), Tone::Success);
        assert_eq!(issue_tone(&issue(4, "closed", "refused", "")), Tone::Error);
        assert_eq!(issue_tone(&issue(4, "closed", "waiting", "")), Tone::Muted);
        let lines = issue_lines(
            &issue(4, "open", "waiting", "VirusTotal is still analysing od-1.0.0.zip."),
            false,
            "a",
            "b",
        );
        assert_eq!(
            lines,
            vec![
                (Tone::Muted, "#4 is open: the catalog checks it.".to_owned()),
                (Tone::Warning, "VirusTotal is still analysing od-1.0.0.zip.".to_owned())
            ]
        );
        assert_eq!(
            issue_lines(&issue(4, "closed", "accepted", ""), true, "was in.", "comes in."),
            vec![(Tone::Success, "#4 was in.".to_owned())]
        );
        assert_eq!(
            issue_lines(&issue(4, "closed", "", ""), true, "a", "b"),
            vec![(Tone::Muted, "#4 was closed.".to_owned())]
        );
    }

    #[test]
    fn the_submit_button_says_what_it_does() {
        let mut v = View::default();
        assert_eq!(submit_label(&v), "Scan and Submit");
        v.virustotal.state = "clean".to_owned();
        assert_eq!(submit_label(&v), "Submit");
        v.submission = issue(9, "open", "", "");
        assert_eq!(submit_label(&v), "Update Submission");
        v.virustotal.state = "analysing".to_owned();
        assert_eq!(submit_label(&v), "Scan and Update");
        assert_eq!(stop_label(Job::Withdraw), "Stop Withdrawing");
        assert_eq!(stop_label(Job::Check), "Stop Checking");
    }

    #[test]
    fn owners() {
        assert_eq!(owner("me/od"), "me");
        assert_eq!(owner(""), "");
    }
}
