//! Mod > Package Release... (docs/UI_DESIGN.md, section 6, "Build dialogs"): the M dialog with
//! the release, its output folder and required game version under More, and the author key row;
//! the `dlg.replace_release` question before an earlier packaging of the version is replaced;
//! `dlg.package_result` when the job ends.
//!
//! The key row ([`KeyLine`], shared
//! with Submit to Mod Browser). The packaging is `gz-release`'s job; `mod.package` is its MCP
//! path, which asks nothing.

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use egui::Ui;
use gz_app::{DialogDesc, DocCx, ShellBuilder};
use gz_editor::{Editor, JobId};
use gz_mod::{Manifest, ManifestService};
use gz_project::Project;
use gz_release::format::{check_requires_game, release_gate, resolve_release_dir};
use gz_release::{KeyState, PackageRequest, PackageResult, Releases, packaged_files};
use gz_ui::dialog::{ConfirmSpec, ResultAction, ResultSpec, confirm, notice};
use gz_ui::grid::{path_field, read_only};
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{Button, Dialog, DialogSize, Kind, PathAction, PropFlags, PropertyGrid, TipExt, Tone, Tx, tr};
use parking_lot::Mutex;

use crate::build::{add_bake_note, asker, build_running, split_lines};
use crate::call::{self, Pick, args, post_ui, s};
use crate::local::local_value_column;

/// The dialog's MCP id.
pub const PACKAGE_RELEASE: &str = "package_release";

// ---- the author key row ------------------------------------------------------------------------

#[derive(Default)]
struct KeyInbox {
    state: Option<KeyState>,
    problem: Option<String>,
}

/// The author key as one row: what signs the release, or why
/// nothing will; a protected key asks for its password in the row, once per session. The state
/// is read on the IO pool when asked ([`KeyLine::refresh`]), never by a frame.
#[derive(Default)]
pub struct KeyLine {
    state: Option<KeyState>,
    password: String,
    problem: String,
    unlocking: bool,
    inbox: Arc<Mutex<KeyInbox>>,
}

impl KeyLine {
    /// Reads the key's state again.
    pub fn refresh(&mut self, editor: &Arc<Editor>) {
        let (ed, inbox) = (editor.clone(), self.inbox.clone());
        let _task = editor.exec.spawn(async move {
            let state = gz_release::key_state(&ed).await;
            inbox.lock().state = Some(state);
            call::wake_ui(&ed);
        });
    }

    /// The key is protected and its password was not given yet this session.
    #[must_use]
    pub fn wants_password(&self) -> bool {
        self.state.as_ref().is_some_and(|k| k.exists && k.error.is_empty() && k.locked)
    }

    fn take(&mut self) {
        let mut inbox = self.inbox.lock();
        if let Some(state) = inbox.state.take() {
            self.state = Some(state);
        }
        if let Some(problem) = inbox.problem.take() {
            self.unlocking = false;
            self.problem = problem;
        }
    }

    fn unlock(&mut self, editor: &Arc<Editor>) {
        // the only copy of the password goes with the request; the field is wiped
        let mut bytes = std::mem::take(&mut self.password).into_bytes();
        let password = String::from_utf8_lossy(&bytes).into_owned();
        bytes.fill(0);
        self.unlocking = true;
        let (ed, inbox) = (editor.clone(), self.inbox.clone());
        let a = args([("action", s("open")), ("password", s(&password))]);
        drop(password);
        let _task = editor.exec.spawn(async move {
            let reply = ed.call("release.key", a, ed.ui_origin()).await;
            let problem = if call::ok(&reply) { String::new() } else { call::error_text(&reply) };
            let state = gz_release::key_state(&ed).await;
            let mut inbox = inbox.lock();
            inbox.problem = Some(problem);
            inbox.state = Some(state);
            drop(inbox);
            call::wake_ui(&ed);
        });
    }

    /// Draws the row's value: a sentence, or the password field and Unlock.
    pub fn draw(&mut self, ui: &mut Ui, editor: &Arc<Editor>, id_salt: &str) {
        self.take();
        let Some(state) = self.state.clone() else {
            gz_ui::state::busy_line(ui, "Reading the author key...");
            return;
        };
        if !state.exists {
            let _ = text::status(
                ui,
                Tone::Warning,
                "Not signed: no author key. Submit to Mod Browser creates one.",
            );
        } else if !state.error.is_empty() {
            let _ = text::status(
                ui,
                Tone::Error,
                tr!("Not signed: the key file does not read: {why}.", why = Tx::ui(state.error.clone())),
            );
        } else if state.locked {
            let mut unlock = false;
            local_value_column(ui, id_salt, |ui| {
                ui.horizontal(|ui| {
                    let button = gz_ui::units::button_width(ui.ctx(), "Unlock", Kind::Secondary, false, 0.0);
                    let width = (ui.available_width() - button - 8.0).max(em(6.0));
                    let field_id = ui.make_persistent_id(("gz-ui-mod.password", id_salt));
                    let field = gz_ui::form::secret_field(ui, field_id, &mut self.password, "Author key password", width)
                        .tip("The author key file is protected. Its password is asked once per session and never stored.");
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    let has = !self.password.is_empty();
                    let clicked = Button::new("Unlock").enabled(has && !self.unlocking, "Type the key file's password first.").show(ui).clicked();
                    unlock = has && !self.unlocking && (clicked || enter);
                });
                if !self.problem.is_empty() {
                    let _ = text::status(ui, Tone::Error, Tx::ui(self.problem.as_str()));
                }
            });
            if unlock {
                self.unlock(editor);
            }
        } else {
            let _ = text::status(
                ui,
                Tone::Success,
                tr!("Signed with the author key {key}.", key = &state.key_id),
            );
        }
    }
}

// ---- the dialog ---------------------------------------------------------------------------------

#[derive(Default)]
struct PackageInbox {
    requires: Option<String>,
    replace: Option<(String, Vec<String>)>,
    folder: Option<String>,
    /// The author agreed to replace the earlier packaging.
    go: bool,
}

#[derive(Default)]
struct PackageDialog {
    open: bool,
    manifest: Manifest,
    out_dir: String,
    requires: String,
    requires_read: bool,
    checking: bool,
    key: KeyLine,
    inbox: Arc<Mutex<PackageInbox>>,
}

/// The release gate's refusal as the result the window shows, Mod Settings its action.
fn cannot_package(editor: &Arc<Editor>, ctx: &egui::Context, err: &str) {
    let headline = tr!("build" => "{why}.", why = Tx::ui(err.to_owned()));
    let mut spec = ResultSpec::new("package_result", "Cannot Package", Tone::Error, headline);
    let ed = editor.clone();
    spec.action = Some(ResultAction {
        label: "Mod Settings".into(),
        run: Arc::new(move || call::open_tab(&ed, crate::settings::TAB)),
    });
    crate::build::show_result(editor, ctx, spec);
}

/// Package Release... from the menu or the toolbar: a build that runs
/// brings its card up, a manifest the release gate refuses is the result, else the dialog.
pub fn request(cx: &mut DocCx<'_>) {
    if cx.editor.project_opt().is_none() {
        return;
    }
    if build_running(cx.editor) {
        cx.requests.task_card(true);
        return;
    }
    let ed = cx.editor.clone();
    let _task = cx.editor.exec.spawn(async move {
        let gate = match ManifestService::of(&ed).await {
            Ok(service) => {
                let _ = service.sync().await;
                match service.snapshot().manifest() {
                    Some(m) => release_gate(m),
                    None => Err("mod.ltx not found in the project".to_owned()),
                }
            }
            Err(e) => Err(e.message),
        };
        let ed2 = ed.clone();
        post_ui(&ed, move |ctx| match gate {
            Err(e) => cannot_package(&ed2, ctx, &e),
            Ok(()) => call::call_then(
                &ed2,
                "ui.dialog",
                args([("action", s("open")), ("id", s(PACKAGE_RELEASE))]),
                |reply, ctx| {
                    if !call::ok(reply) {
                        gz_ui::state::toast(ctx, Tone::Error, Tx::ui(call::error_text(reply)));
                    }
                },
            ),
        });
    });
}

/// What a packaging did.
#[must_use]
pub fn package_report(m: &Manifest, r: &PackageResult) -> ResultSpec {
    let mut rows = Vec::new();
    let mut warnings = Vec::new();
    if !r.ok {
        add_bake_note(&r.baked, &mut rows, &mut warnings);
        let mut spec =
            ResultSpec::new("package_result", "Packaging Failed", Tone::Error, Tx::ui(r.err.clone()));
        spec.rows = rows;
        spec.warnings = warnings;
        return spec;
    }
    rows.push(("Folder".into(), Tx::raw(r.out_dir.clone())));
    rows.push(("Descriptor".into(), Tx::raw(format!("{}.update.ltx", m.id))));
    rows.push(("File index".into(), Tx::raw(r.index.name.clone())));
    for p in &r.packages {
        let size = format!("{:.1}", p.size as f64 / 1_048_576.0);
        rows.push(("Package".into(), tr!("{name}  ({size} MB)", name = &p.name, size = size)));
    }
    rows.push((
        "Signed".into(),
        if r.signed {
            tr!("With the author key {key}", key = &r.key_id)
        } else {
            Tx::from("No - the Mod Browser lists signed releases only")
        },
    ));
    add_bake_note(&r.baked, &mut rows, &mut warnings);
    if m.update_github.is_empty() {
        warnings.push(
            "No GitHub repository in Mod Settings: the game does not look for updates of this module."
                .to_owned(),
        );
    }
    split_lines(&r.warnings, &mut warnings);
    // the index addresses the zip by byte offset: a repacked copy is another file
    let headline = if m.update_github.is_empty() {
        tr!(
            "Release {id} {version}: {n} {n:file|files} packaged. Upload the files exactly as written to a GitHub release and mark it as the latest - Publish Release does both in one step.",
            id = &m.id,
            version = &m.version,
            n = r.files
        )
    } else {
        tr!(
            "Release {id} {version}: {n} {n:file|files} packaged. Upload the files exactly as written to a GitHub release of {repo} and mark it as the latest - Publish Release does both in one step.",
            id = &m.id,
            version = &m.version,
            n = r.files,
            repo = &m.update_github
        )
    };
    let tone = if warnings.is_empty() { Tone::Success } else { Tone::Warning };
    let mut spec = ResultSpec::new("package_result", "Release Packaged", tone, headline);
    spec.rows = rows;
    spec.left_out.clone_from(&r.left_out);
    spec.warnings = warnings;
    let dir = r.out_dir.clone();
    spec.action =
        Some(ResultAction { label: "Open Folder".into(), run: Arc::new(move || call::open_folder(&dir)) });
    spec
}

/// Starts the packaging job; its end raises the result.
fn queue_package(editor: &Arc<Editor>, ctx: &egui::Context, m: Manifest, out_dir: String, requires: String) {
    match Releases::start(editor, PackageRequest { out_dir, requires_game: Some(requires) }) {
        Ok(job) => {
            crate::cards::started_from_ui(editor, job);
            follow_package(editor, job, m);
        }
        Err(e) => call::log_lines(
            editor,
            notice(
                ctx,
                asker(editor),
                Tone::Warning,
                "Packaging Release",
                tr!("build" => "{why}.", why = Tx::ui(e.message)),
            ),
        ),
    }
}

fn follow_package(editor: &Arc<Editor>, job: JobId, m: Manifest) {
    let ed = editor.clone();
    let _task = editor.exec.spawn(async move {
        let _ = ed.jobs.wait(job).await;
        let r = Releases::of(&ed).result(job);
        let ed2 = ed.clone();
        post_ui(&ed, move |ctx| match r {
            Some(r) if r.cancelled => gz_ui::state::toast(
                ctx,
                Tone::Info,
                "Packaging stopped: the release folder holds nothing new.",
            ),
            Some(r) => crate::build::show_result(&ed2, ctx, package_report(&m, &r)),
            None => {
                let spec = ResultSpec::new(
                    "package_result",
                    "Packaging Failed",
                    Tone::Error,
                    "The packaging ended without an answer.",
                );
                crate::build::show_result(&ed2, ctx, spec);
            }
        });
    });
}

impl PackageDialog {
    fn open(&mut self, cx: &mut DocCx<'_>) -> Result<(), String> {
        let (session, project) = Project::of(cx.editor).map_err(|e| e.message)?;
        if build_running(cx.editor) {
            cx.requests.task_card(true);
            return Err("a build is running: its card in the Task Center follows it".to_owned());
        }
        let Some(service) = ManifestService::get(&session) else {
            let ed = cx.editor.clone();
            let _task = cx.editor.exec.spawn(async move {
                let _ = ManifestService::of(&ed).await;
            });
            return Err("mod.ltx is still being read: open the dialog again".to_owned());
        };
        let snap = service.snapshot();
        let Some(m) = snap.manifest() else { return Err("mod.ltx not found in the project".to_owned()) };
        release_gate(m)?;
        self.manifest = m.clone();
        self.out_dir = format!("{}\\releases\\{}", project.root(), m.version);
        self.requires.clear();
        self.requires_read = false;
        self.checking = false;
        self.open = true;
        // absent from the dialog = the linked game's version, read off its xrEngine.exe
        let (ed, inbox, game) = (cx.editor.clone(), self.inbox.clone(), project.game_root());
        let _task = cx.editor.exec.spawn(async move {
            let version = ed.exec.io(move || gz_release::game_version::linked_game_version(&game)).await;
            inbox.lock().requires = Some(version);
            call::wake_ui(&ed);
        });
        self.key.refresh(cx.editor);
        Ok(())
    }

    /// Package: an earlier packaging of this version is replaced only once the author has seen
    /// which files that is.
    fn check_replace(&mut self, editor: &Arc<Editor>) {
        let Some(root) = editor.project_root() else { return };
        self.checking = true;
        let (out_dir, id, version) =
            (self.out_dir.clone(), self.manifest.id.clone(), self.manifest.version.clone());
        let (ed, inbox) = (editor.clone(), self.inbox.clone());
        let _task = editor.exec.spawn(async move {
            let found = ed
                .exec
                .io(move || {
                    let dir = resolve_release_dir(Path::new(&root), &out_dir, &version).unwrap_or_default();
                    (dir, packaged_files(Path::new(&root), &out_dir, &id, &version))
                })
                .await;
            inbox.lock().replace = Some(found);
            call::wake_ui(&ed);
        });
    }

    fn draw(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        if !self.open {
            return;
        }
        let editor = cx.editor.clone();
        {
            let mut inbox = self.inbox.lock();
            if let Some(v) = inbox.requires.take() {
                self.requires = v;
                self.requires_read = true;
            }
            if let Some(folder) = inbox.folder.take() {
                self.out_dir = folder;
            }
            if std::mem::take(&mut inbox.go) {
                drop(inbox);
                // the dialog closes when the job starts: the Task Center's card follows it
                self.open = false;
                let (m, out, req) =
                    (self.manifest.clone(), self.out_dir.trim().to_owned(), self.requires.trim().to_owned());
                queue_package(&editor, cx.ctx, m, out, req);
                return;
            }
        }
        let running = build_running(&editor);
        let requires_ok = check_requires_game(self.requires.trim()).is_ok();
        let can = !self.out_dir.trim().is_empty()
            && requires_ok
            && !running
            && self.requires_read
            && !self.checking;
        let why = if running {
            "A build is running"
        } else if self.out_dir.trim().is_empty() {
            "Pick the output folder first"
        } else if !self.requires_read {
            "Reading the linked game's version"
        } else if self.checking {
            "Looking at the release folder"
        } else {
            "Fix the game version first"
        };
        let release = format!("{}  {}", self.manifest.id, self.manifest.version);
        let no_repo = self.manifest.update_github.is_empty();
        let wants_password = self.key.wants_password();
        let mut package = false;
        let mut cancel = false;
        let mut browse = false;
        let mut start_now = false;
        let asked_by = asker(&editor);
        let Self { open, out_dir, requires, key, inbox, manifest, checking, .. } = self;
        let _ = Dialog::new("Package Release", "dlg.package_release", DialogSize::M).show(
            ui,
            open,
            |ui| {
                // a question raised here is a sheet of this dialog
                let found = inbox.lock().replace.take();
                if let Some((dir, files)) = found {
                    *checking = false;
                    if files.is_empty() {
                        start_now = true;
                    } else {
                        let spec = ConfirmSpec {
                            id: "replace_release".to_owned(),
                            title: "Replace Release Files".into(),
                            headline: tr!("An earlier packaging of {version} is in {dir}.", version = &manifest.version, dir = dir),
                            items: files.into_iter().map(Tx::raw).collect(),
                            total: 0,
                            consequence: "Deleted for good - this cannot be undone. The new packaging takes their place.".into(),
                            action: "Replace".into(),
                            destructive: true,
                        };
                        let (ed, inbox) = (editor.clone(), inbox.clone());
                        let line = confirm(ui.ctx(), asked_by, spec, move |answer| {
                            if answer == gz_ui::Answer::Yes {
                                inbox.lock().go = true;
                                call::wake_ui(&ed);
                            }
                        });
                        call::log_lines(&editor, line);
                    }
                }
                // the label column is as wide as the longest label in the window's language
                let label_w = ["Release", "Output Folder", "Requires Game", "Author Key"]
                    .iter()
                    .map(|l| text::width(ui.ctx(), &gz_ui::Font::Body.id(), *l) + em(1.0))
                    .fold(em(9.0), f32::max);
                PropertyGrid::new("package_dialog").label_frac(label_w / ui.available_width().max(1.0)).show(ui, |g| {
                    g.row("Release", "The module id and the version from Mod Settings.", PropFlags::READ_ONLY, 0, |ui| read_only(ui, Tx::raw(&release), Clip::End));
                    g.section("More", out_dir.trim().is_empty() || !requires_ok, None, |g| {
                        let action = g.row(
                            "Output Folder",
                            "Receives <id>.update.ltx, the file index and the package zip(s). Inside the project only releases\\ is allowed: anywhere else the packages would ship with the next build.",
                            PropFlags::REQUIRED,
                            1,
                            |ui| path_field(ui, "out_dir", out_dir, true),
                        );
                        if action == Some(PathAction::Browse) {
                            browse = true;
                        }
                        g.row(
                            "Requires Game",
                            "The lowest Dead Air: Refined version this release runs on. Empty: any.",
                            if requires_ok { PropFlags::NONE } else { PropFlags::INVALID },
                            1,
                            |ui| {
                                ui.set_max_width(em(8.0).min(ui.available_width()));
                                gz_ui::form::row_field(ui, ("gz-ui-mod.text", "requires_game"), requires, "Any")
                            },
                        );
                        if !requires_ok {
                            g.message(Tone::Error, "MAJOR.MINOR.PATCH, e.g. 1.4.1.");
                        }
                    });
                    // read-only unless the key asks for its password: a locked key only leaves the release unsigned
                    g.row(
                        "Author Key",
                        "The key file that signs the release: the Mod Browser lists signed releases only. Submit to Mod Browser creates one.",
                        if wants_password { PropFlags::NONE } else { PropFlags::READ_ONLY },
                        0,
                        |ui| key.draw(ui, &editor, "package"),
                    );
                });
                if no_repo {
                    ui.add_space(4.0);
                    let _ = text::status(ui, Tone::Warning, "No GitHub repository in Mod Settings: the game will not find this release.");
                }
            },
            |f| {
                if f.button("Package", Kind::Primary, true, can, why) {
                    package = true;
                }
                if f.button("Cancel", Kind::Secondary, false, true, "") {
                    cancel = true;
                }
            },
        );
        if cancel {
            self.open = false;
        }
        if browse {
            let inbox = self.inbox.clone();
            call::pick_path(&editor, Pick::Folder, "Package Release to", &self.out_dir, move |folder, _| {
                inbox.lock().folder = Some(folder);
            });
        }
        if package {
            self.check_replace(&editor);
        }
        if start_now {
            // the dialog closes when the job starts: the Task Center's card follows it
            self.open = false;
            let (m, out, req) =
                (self.manifest.clone(), self.out_dir.trim().to_owned(), self.requires.trim().to_owned());
            queue_package(&editor, cx.ctx, m, out, req);
        }
    }
}

pub(crate) fn install(b: &mut ShellBuilder) {
    if !b.releases() {
        return;
    }
    let state = Rc::new(RefCell::new(PackageDialog::default()));
    let (open_state, draw_state) = (state.clone(), state);
    b.dialog(DialogDesc {
        id: PACKAGE_RELEASE.into(),
        window: "Package Release###dlg.package_release".into(),
        needs_project: true,
        open: Box::new(move |cx| open_state.borrow_mut().open(cx)),
        draw: Box::new(move |ui, cx| draw_state.borrow_mut().draw(ui, cx)),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_release::Asset;

    #[test]
    fn a_release_lists_what_it_wrote() {
        let m = Manifest { id: "od".into(), version: "1.2.0".into(), ..Manifest::default() };
        let r = PackageResult {
            ok: true,
            out_dir: "P\\releases\\1.2.0".into(),
            files: 2,
            packages: vec![Asset { name: "od-1.2.0.zip".into(), size: 1_048_576, ..Asset::default() }],
            ..PackageResult::default()
        };
        let spec = package_report(&m, &r);
        assert_eq!(spec.title.en(), "Release Packaged");
        // no repository: a warning, so the tone is not Success
        assert_eq!(spec.tone, Tone::Warning);
        assert!(spec.rows.iter().any(|(k, v)| k.en() == "Package" && v.en() == "od-1.2.0.zip  (1.0 MB)"));
        assert!(spec.headline.en().starts_with("Release od 1.2.0: 2 files packaged."));
        assert!(spec.rows.iter().any(|(k, v)| k.en() == "Signed" && v.en().starts_with("No")));
    }

    #[test]
    fn a_failed_packaging_says_why() {
        let r = PackageResult { ok: false, err: "nothing to pack".into(), ..PackageResult::default() };
        let spec = package_report(&Manifest::default(), &r);
        assert_eq!((spec.title.en().as_ref(), spec.tone), ("Packaging Failed", Tone::Error));
    }
}
