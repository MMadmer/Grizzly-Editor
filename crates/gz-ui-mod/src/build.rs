//! The builds started from the window (docs/UI_DESIGN.md, section 6, "Build dialogs"): Build
//! Into Game, the Build to Folder and Export Flat Gamedata dialogs, the clean-target question and
//! the result.
//!
//! A build runs beside the editor as `gz-build`'s job: the dialog closes when it starts, the
//! Task Center's card follows it ([`crate::cards`]), and its end raises the kit's one result
//! dialog. A clean build that would delete files it did not write refuses first; the window
//! then asks `dlg.clean_target`, naming up to ten of them, and Yes builds again with `clean`.
//! MCP builds (`mod.build`) never come here: they ask nothing and open nothing.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use egui::Ui;
use gz_app::{DialogDesc, DocCx, ShellBuilder};
use gz_build::{BuildKind, BuildRequest, BuildResult, Builds, CleanQuestion};
use gz_editor::{Editor, JobId};
use gz_mod::{ManifestService, validate_id};
use gz_project::Project;
use gz_ui::dialog::{Asker, ConfirmSpec, ResultAction, ResultSpec, confirm, notice, result};
use gz_ui::grid::{PathAction, path_field, read_only};
use gz_ui::text::{self, Clip};
use gz_ui::{Dialog, DialogSize, Icon, Kind, PropFlags, PropertyGrid, Tone, Tx, tr};
use parking_lot::Mutex;

use crate::call::{self, Pick, post_ui};
use crate::form::writes_to;

/// The MCP ids of the two dialogs.
pub const BUILD_TO_FOLDER: &str = "build_to_folder";
pub const EXPORT_FLAT: &str = "export_flat";

const MISSING_ID: &str =
    "The module id is missing or invalid. Set it in Mod Settings: a-z, 0-9, _ . and - only.";

/// Who answers the window's questions: a person, or `-nodlg`'s logged answers.
pub(crate) fn asker(editor: &Editor) -> Asker {
    if editor.flags.unattended { Asker::Unattended } else { Asker::Ui }
}

/// The last result of each kind the window showed (or, under `-nodlg`, logged): what
/// `ui.dialog open id=build_result` puts up again to be looked at.
#[derive(Default)]
struct LastResults(Mutex<BTreeMap<String, ResultSpec>>);

/// Shows a result as the window does - a person sees the dialog, `-nodlg` logs it - and keeps it
/// to be looked at again.
pub(crate) fn show_result(editor: &Arc<Editor>, ctx: &egui::Context, spec: ResultSpec) {
    editor.services.get_or_init(LastResults::default).0.lock().insert(spec.id.clone(), spec.clone());
    call::log_lines(editor, result(ctx, asker(editor), spec));
}

/// The result last shown of `id`; for a run MCP started, the one its job left.
fn last_result(editor: &Editor, id: &str) -> Option<ResultSpec> {
    if let Some(spec) = editor.services.get::<LastResults>().and_then(|l| l.0.lock().get(id).cloned()) {
        return Some(spec);
    }
    let manifest = editor
        .project_opt()
        .and_then(|s| ManifestService::get(&s))
        .and_then(|svc| svc.snapshot().manifest().cloned())
        .unwrap_or_default();
    match id {
        "build_result" | "export_result" => {
            let r = Builds::of(editor).last()?;
            let flat = std::path::Path::new(&r.path)
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("gamedata_"));
            let spec =
                build_report(if flat { BuildKind::Flat } else { BuildKind::ToFolder }, &r, &manifest.id);
            (spec.id == id).then_some(spec)
        }
        "package_result" => {
            gz_release::Releases::of(editor).last().map(|r| crate::package::package_report(&manifest, &r))
        }
        _ => None,
    }
}

/// A build or a packaging runs now.
#[must_use]
pub fn build_running(editor: &Editor) -> bool {
    editor.jobs.running_in("build").is_some()
}

/// The module id `mod.ltx` holds now, from the store's snapshot; `None` before the store is read.
#[must_use]
pub fn manifest_id(editor: &Editor) -> Option<String> {
    let session = editor.project_opt()?;
    let service = ManifestService::get(&session)?;
    let snap = service.snapshot();
    Some(snap.manifest().map(|m| m.id.clone()).unwrap_or_default())
}

/// The linked game's folder; empty without one.
#[must_use]
pub fn game_root(editor: &Editor) -> String {
    Project::of(editor).map(|(_, p)| p.game_root()).unwrap_or_default()
}

/// A project is open and a game is linked.
#[must_use]
pub fn can_build_into_game(editor: &Editor) -> bool {
    !game_root(editor).is_empty()
}

/// Where Build Into Game writes: `<game>\modules\<id>`; empty without a game or a valid id.
#[must_use]
pub fn build_target_text(editor: &Editor) -> String {
    let root = game_root(editor);
    match manifest_id(editor) {
        Some(id) if !root.is_empty() && validate_id(&id) => writes_to(&root, &id, false),
        _ => String::new(),
    }
}

/// What the running build or packaging is doing, for a tip: "Building Module: Copying the files".
#[must_use]
pub fn job_status_text(editor: &Editor) -> Tx<'static> {
    let job = editor.jobs.running_in("build");
    let Some(job) = job else { return Tx::EMPTY };
    if job.stage.is_empty() {
        Tx::ui(job.title)
    } else {
        tr!("build" => "{title}: {stage}", title = Tx::ui(job.title), stage = Tx::ui(job.stage))
    }
}

/// Reads the manifest's store (made on first use), then hands its module id to `then` on the UI
/// thread.
fn with_manifest_id(
    editor: &Arc<Editor>,
    then: impl FnOnce(&Arc<Editor>, Option<String>, &egui::Context) + Send + 'static,
) {
    let ed = editor.clone();
    let _task = editor.exec.spawn(async move {
        let id = match ManifestService::of(&ed).await {
            Ok(service) => {
                let _ = service.sync().await;
                Some(service.snapshot().manifest().map(|m| m.id.clone()).unwrap_or_default())
            }
            Err(_) => None,
        };
        let ed2 = ed.clone();
        post_ui(&ed, move |ctx| then(&ed2, id, ctx));
    });
}

/// The result naming a missing module id, with Mod Settings as its action.
pub(crate) fn report_missing_id(editor: &Arc<Editor>, ctx: &egui::Context, title: &'static str) {
    let mut spec = ResultSpec::new("build_result", title, Tone::Error, MISSING_ID);
    let ed = editor.clone();
    spec.action = Some(ResultAction {
        label: "Mod Settings".into(),
        run: Arc::new(move || call::open_tab(&ed, crate::settings::TAB)),
    });
    show_result(editor, ctx, spec);
}

/// What the scene's bake did: its notes go to the rows of the result, what it left out or
/// failed at to its warnings.
pub(crate) fn add_bake_note(
    note: &[gz_base::Finding],
    rows: &mut Vec<(Tx<'static>, Tx<'static>)>,
    warnings: &mut Vec<String>,
) {
    for f in note {
        match f.severity {
            gz_base::Severity::Note => rows.push(("Scene".into(), Tx::ui(f.text.clone()))),
            _ => warnings.push(f.text.clone()),
        }
    }
}

/// The lines of a warnings text, empty ones left out.
pub(crate) fn split_lines(text: &str, out: &mut Vec<String>) {
    out.extend(text.lines().map(str::trim_end).filter(|l| !l.is_empty()).map(str::to_owned));
}

/// The footnote of a result whose module works with the network: what the game does about it. A
/// fact for the author - never a warning, and nothing a log calls a problem.
#[must_use]
pub fn network_note(places: &[String]) -> Option<Tx<'static>> {
    let first = places.first()?;
    Some(if places.len() > 1 {
        tr!(
            "The module's code works with the network ({place} and {n} more). The game marks it Network access in the Mods menu and the Mod Browser and says so before a player installs it.",
            place = Tx::raw(first.clone()),
            n = places.len() - 1
        )
    } else {
        tr!(
            "The module's code works with the network ({place}). The game marks it Network access in the Mods menu and the Mod Browser and says so before a player installs it.",
            place = Tx::raw(first.clone())
        )
    })
}

/// What a build or an export did, in the result dialog every build shares.
#[must_use]
pub fn build_report(kind: BuildKind, r: &BuildResult, id: &str) -> ResultSpec {
    let flat = kind == BuildKind::Flat;
    let mut rows = Vec::new();
    let mut warnings = Vec::new();
    if r.ok {
        rows.push(("Folder".into(), Tx::raw(r.path.clone())));
    }
    add_bake_note(&r.bake_note, &mut rows, &mut warnings);
    split_lines(&r.warnings, &mut warnings);
    let result_id = if flat { "export_result" } else { "build_result" };
    if !r.ok {
        let mut spec = ResultSpec::new(
            result_id,
            if flat { "Export Failed" } else { "Build Failed" },
            Tone::Error,
            Tx::ui(r.err.clone()),
        );
        spec.rows = rows;
        spec.warnings = warnings;
        return spec;
    }
    let n = r.files;
    let headline = match kind {
        BuildKind::IntoGame => {
            rows.push(("Console".into(), Tx::raw(format!("xms_enable {id}  /  xms_disable {id}"))));
            tr!(
                "The module is in the game: {n} {n:file|files}. Added objects reach new games and loaded saves; changes to existing objects need a new game.",
                n = n
            )
        }
        BuildKind::Flat => tr!("{n} {n:file|files} exported as a gamedata folder.", n = n),
        BuildKind::ToFolder => tr!("The module is built: {n} {n:file|files}.", n = n),
    };
    let tone = if warnings.is_empty() { Tone::Success } else { Tone::Warning };
    let mut spec =
        ResultSpec::new(result_id, if flat { "Export Complete" } else { "Build Complete" }, tone, headline);
    spec.rows = rows;
    spec.left_out.clone_from(&r.left_out);
    spec.warnings = warnings;
    spec.notes.extend(network_note(&r.network));
    let path = r.path.clone();
    spec.action =
        Some(ResultAction { label: "Open Folder".into(), run: Arc::new(move || call::open_folder(&path)) });
    spec
}

/// The clean-target question as the kit's confirmation.
#[must_use]
pub fn clean_confirm(q: &CleanQuestion) -> ConfirmSpec {
    ConfirmSpec {
        id: q.id.to_owned(),
        title: Tx::ui(q.title),
        headline: Tx::ui(q.headline.clone()),
        items: q.items.iter().map(|i| Tx::raw(i.clone())).collect(),
        total: q.total,
        consequence: Tx::ui(q.consequence),
        action: Tx::ui(q.action),
        destructive: true,
    }
}

/// Starts a build of the open project from the window: the module id first, then
/// the job; its end raises the clean-target question or the result.
pub fn start_build(editor: &Arc<Editor>, kind: BuildKind, target: String, clean: bool) {
    with_manifest_id(editor, move |ed, id, ctx| {
        let flat = kind == BuildKind::Flat;
        let Some(id) = id.filter(|i| validate_id(i)) else {
            report_missing_id(ed, ctx, if flat { "Export Flat Gamedata" } else { "Build Failed" });
            return;
        };
        let title = if flat { "Exporting Gamedata" } else { "Building Module" };
        match Builds::start(ed, BuildRequest { kind, target: target.clone(), clean, remember_target: true }) {
            Ok(job) => {
                crate::cards::started_from_ui(ed, job);
                follow_build(ed, job, kind, target, id);
            }
            Err(e) => call::log_lines(
                ed,
                notice(
                    ctx,
                    asker(ed),
                    Tone::Warning,
                    title,
                    tr!("build" => "{why}.", why = Tx::ui(e.message)),
                ),
            ),
        }
    });
}

/// Waits for the build job and answers on the UI thread.
fn follow_build(editor: &Arc<Editor>, job: JobId, kind: BuildKind, target: String, id: String) {
    let ed = editor.clone();
    let _task = editor.exec.spawn(async move {
        let _ = ed.jobs.wait(job).await;
        let r = Builds::of(&ed).result(job);
        let ed2 = ed.clone();
        post_ui(&ed, move |ctx| build_ended(&ed2, ctx, kind, target, &id, r));
    });
}

fn build_ended(
    editor: &Arc<Editor>,
    ctx: &egui::Context,
    kind: BuildKind,
    target: String,
    id: &str,
    r: Option<Arc<BuildResult>>,
) {
    let flat = kind == BuildKind::Flat;
    let Some(r) = r else {
        let spec = ResultSpec::new(
            if flat { "export_result" } else { "build_result" },
            if flat { "Export Failed" } else { "Build Failed" },
            Tone::Error,
            "The build ended without an answer.",
        );
        show_result(editor, ctx, spec);
        return;
    };
    if r.cancelled {
        let text = if flat {
            "Export stopped: the folder holds the files written so far."
        } else {
            "Build stopped: the module stays as it was."
        };
        gz_ui::state::toast(ctx, Tone::Info, text);
        return;
    }
    if let Some(q) = CleanQuestion::of(&r) {
        let ed = editor.clone();
        let line = confirm(ctx, asker(editor), clean_confirm(&q), move |answer| {
            if answer == gz_ui::Answer::Yes {
                start_build(&ed, kind, target, true);
            } else {
                post_ui(&ed, |ctx| {
                    gz_ui::state::toast(ctx, Tone::Info, "Build cancelled: nothing was deleted.")
                });
            }
        });
        call::log_lines(editor, line);
        return;
    }
    show_result(editor, ctx, build_report(kind, &r, id));
}

/// Build Into Game from the menu, the shortcut or the toolbar: a build
/// that runs brings its card up; without a game the result says so.
pub fn request_build_into_game(cx: &mut DocCx<'_>) {
    if build_running(cx.editor) {
        cx.requests.task_card(true);
        return;
    }
    let root = game_root(cx.editor);
    if root.is_empty() {
        let spec = ResultSpec::new(
            "build_result",
            "Build Failed",
            Tone::Error,
            "No game is linked. The build writes the module into <game>\\modules\\<id>, where the game reads it: link a game first.",
        );
        show_result(cx.editor, cx.ctx, spec);
        return;
    }
    start_build(cx.editor, BuildKind::IntoGame, root, false);
}

/// Build to Folder... or Export Flat Gamedata... from the menu or the toolbar:
/// a build that runs brings its card up, a missing module id is the result, else the dialog.
pub fn request_export(cx: &mut DocCx<'_>, flat: bool) {
    if cx.editor.project_opt().is_none() {
        return;
    }
    if build_running(cx.editor) {
        cx.requests.task_card(true);
        return;
    }
    with_manifest_id(cx.editor, move |ed, id, ctx| {
        if !id.is_some_and(|i| validate_id(&i)) {
            report_missing_id(ed, ctx, if flat { "Export Flat Gamedata" } else { "Build to Folder" });
            return;
        }
        let dialog = if flat { EXPORT_FLAT } else { BUILD_TO_FOLDER };
        call::call_then(
            ed,
            "ui.dialog",
            call::args([("action", call::s("open")), ("id", call::s(dialog))]),
            |reply, ctx| {
                if !call::ok(reply) {
                    gz_ui::state::toast(ctx, Tone::Error, Tx::ui(call::error_text(reply)));
                }
            },
        );
    });
}

/// The state of one of the two dialogs.
struct ExportDialog {
    flat: bool,
    open: bool,
    target: String,
    /// A folder the system dialog picked.
    picked: Arc<Mutex<Option<String>>>,
    asked_store: bool,
}

impl ExportDialog {
    fn new(flat: bool) -> Self {
        Self {
            flat,
            open: false,
            target: String::new(),
            picked: Arc::new(Mutex::new(None)),
            asked_store: false,
        }
    }

    /// Opens it: the last target, or the linked game the first time.
    fn open(&mut self, cx: &mut DocCx<'_>) -> Result<(), String> {
        let (_, project) = Project::of(cx.editor).map_err(|e| e.message)?;
        if build_running(cx.editor) {
            cx.requests.task_card(true);
            return Err("a build is running: its card in the Task Center follows it".to_owned());
        }
        let remembered = project.export_target();
        // the first export of a linked project: the game root IS the deploy target
        self.target = if remembered.is_empty() { project.game_root() } else { remembered };
        self.open = true;
        self.asked_store = false;
        Ok(())
    }

    fn draw(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        if !self.open {
            return;
        }
        if let Some(folder) = self.picked.lock().take() {
            self.target = folder;
        }
        let editor = cx.editor.clone();
        let flat = self.flat;
        let id = manifest_id(&editor);
        if id.is_none() && !self.asked_store {
            // the store is made on first use; the dialog follows it once it is read
            self.asked_store = true;
            with_manifest_id(&editor, |_, _, _| {});
        }
        let valid_id = id.as_deref().is_some_and(validate_id);
        let root = game_root(&editor);
        let running = build_running(&editor);
        let (title, name) = if flat {
            ("Export Flat Gamedata", "dlg.export_flat")
        } else {
            ("Build to Folder", "dlg.build_to_folder")
        };
        let mut start = false;
        let mut cancel = false;
        let mut browse = false;
        let can = !self.target.trim().is_empty() && !running && valid_id;
        let why = if running {
            "A build is running"
        } else if !valid_id {
            "Set a valid module id in Mod Settings first"
        } else {
            "Pick a folder first"
        };
        let Self { open, target, .. } = self;
        let _ = Dialog::new(title, name, DialogSize::M).show(
            ui,
            open,
            |ui| {
                let _ = text::prose(
                    ui,
                    if flat {
                        "A gamedata_<id> folder is made inside the folder: a JSGME mod, for installs without the module system. Quests, spawns and the game mode need the module build."
                    } else {
                        "The module goes to <folder>\\modules\\<id>, where the game reads modules from. The build is clean: that folder then holds exactly this build."
                    },
                );
                ui.add_space(8.0);
                PropertyGrid::new("build_dialog").label_frac(gz_ui::units::em(8.0) / ui.available_width().max(1.0)).show(ui, |g| {
                    let action = g.row(
                        if flat { "Folder" } else { "Game Folder" },
                        if flat { "The folder the gamedata_<id> folder is made in." } else { "The game folder, or any folder that stands in for one." },
                        PropFlags::REQUIRED,
                        0,
                        |ui| path_field(ui, "target", target, true),
                    );
                    if action == Some(PathAction::Browse) {
                        browse = true;
                    }
                    if !root.is_empty() && g.trailing(Icon::GameFolder, tr!("Use the linked game: {root}", root = root.as_str())) {
                        target.clone_from(&root);
                    }
                    match id.as_deref() {
                        Some(id) if valid_id && !target.trim().is_empty() => {
                            let dest = writes_to(target.trim(), id, flat);
                            g.row("Writes To", "The folder this build fills.", PropFlags::READ_ONLY, 0, |ui| read_only(ui, Tx::raw(&dest), Clip::Start));
                        }
                        Some(_) if !valid_id => {
                            g.message(Tone::Error, MISSING_ID);
                        }
                        None => {
                            g.block(|ui| gz_ui::state::busy_line(ui, "Reading mod.ltx..."));
                        }
                        _ => {}
                    }
                });
            },
            |f| {
                if f.button(if flat { "Export" } else { "Build" }, Kind::Primary, true, can, why) {
                    start = true;
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
            let slot = self.picked.clone();
            call::pick_path(
                &editor,
                Pick::Folder,
                if flat { "Export Flat Gamedata to" } else { "Build to Folder" },
                &self.target,
                move |folder, _| {
                    *slot.lock() = Some(folder);
                },
            );
        }
        if start {
            // the dialog closes when the job starts: the Task Center's card follows it
            self.open = false;
            let kind = if flat { BuildKind::Flat } else { BuildKind::ToFolder };
            start_build(&editor, kind, self.target.trim().to_owned(), false);
        }
    }
}

pub(crate) fn install(b: &mut ShellBuilder) {
    // the results open over MCP only to be looked at again: they show even under -nodlg
    for id in ["build_result", "export_result", "package_result", "bake_result"] {
        b.dialog(DialogDesc {
            id: id.into(),
            window: format!("###dlg.{id}").into(),
            needs_project: false,
            open: Box::new(move |cx| {
                let spec = last_result(cx.editor, id).ok_or_else(|| {
                    format!("no {} to show yet: nothing was built this session", id.replace('_', " "))
                })?;
                // it was logged when it was first shown
                let _ = result(cx.ctx, Asker::Ui, spec);
                Ok(())
            }),
            draw: Box::new(|_, _| {}),
        });
    }
    for flat in [false, true] {
        let state = Rc::new(RefCell::new(ExportDialog::new(flat)));
        let (open_state, draw_state) = (state.clone(), state);
        b.dialog(DialogDesc {
            id: (if flat { EXPORT_FLAT } else { BUILD_TO_FOLDER }).into(),
            window: (if flat {
                "Export Flat Gamedata###dlg.export_flat"
            } else {
                "Build to Folder###dlg.build_to_folder"
            })
            .into(),
            needs_project: true,
            open: Box::new(move |cx| open_state.borrow_mut().open(cx)),
            draw: Box::new(move |ui, cx| draw_state.borrow_mut().draw(ui, cx)),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_into_the_game_names_the_console_commands() {
        let r = BuildResult {
            ok: true,
            files: 3,
            path: "D:\\G\\modules\\od".into(),
            warnings: "one\n".into(),
            bake_note: vec![gz_base::Finding::info("spawn layer: 2"), gz_base::Finding::warning("lost")],
            ..BuildResult::default()
        };
        let spec = build_report(BuildKind::IntoGame, &r, "od");
        assert_eq!(spec.id, "build_result");
        assert_eq!(spec.title.en(), "Build Complete");
        assert_eq!(spec.tone, Tone::Warning);
        assert!(spec.headline.en().starts_with("The module is in the game: 3 files."));
        let keys: Vec<String> = spec.rows.iter().map(|(k, _)| k.en().into_owned()).collect();
        assert_eq!(keys, ["Folder", "Scene", "Console"]);
        assert_eq!(spec.warnings, ["lost", "one"]);
        assert!(spec.action.is_some());
    }

    #[test]
    fn a_failed_export_says_why() {
        let r = BuildResult { ok: false, err: "the target is a link".into(), ..BuildResult::default() };
        let spec = build_report(BuildKind::Flat, &r, "od");
        assert_eq!(
            (spec.id.as_str(), spec.title.en().as_ref(), spec.tone),
            ("export_result", "Export Failed", Tone::Error)
        );
        assert_eq!(spec.headline.en(), "the target is a link");
        assert!(spec.action.is_none());
    }

    #[test]
    fn the_clean_question_cancels_by_default() {
        let r = BuildResult {
            ok: false,
            err: "target already holds a module".into(),
            path: "D:\\G\\modules\\od".into(),
            stale_names: vec!["a.txt".into()],
            stale_count: 12,
            ..BuildResult::default()
        };
        let q = CleanQuestion::of(&r).map(|q| clean_confirm(&q));
        let q = q.as_ref();
        assert_eq!(q.map(|q| q.id.as_str()), Some("clean_target"));
        assert_eq!(q.map(|q| q.total), Some(12));
        assert!(q.is_some_and(|q| q.destructive));
    }
}
