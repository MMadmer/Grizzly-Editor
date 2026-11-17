//! The Publish Release dialog (L, `Publish Release###dlg.publish_release`), the edit dialog of a
//! release (M, `dlg.edit_release`) and the result of a publish started here (the kit's `result`,
//! `dlg.publish_result`).
//!
//! The dialog only SHOWS the module - one Release line (name, version, repository) above the
//! Notes - and never writes `mod.ltx`: Mod Settings edits it. Tag, Title, Options, Requires Game,
//! the author key and the account wait under More unless they are what stops the release. It
//! reads the repository's releases as it opens (Publish stays off until the read ends; a read
//! that fails is only a note) and lists a version that is already out as a problem before the
//! click. It closes when the job starts: the Task Center's card follows it, the result is the
//! kit's result dialog.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use egui::Ui;
use gz_app::{DocCx, OpenRequest};
use gz_editor::Editor;
use gz_github::{AccountSource, Github, ReleaseInfo, Secret};
use gz_mod::ManifestService;
use gz_publish::{
    ManageOp, ManageRequest, ManageState, ManageView, PublishOptions, PublishState, Publisher, ReleaseList,
    SignState, tags,
};
use gz_ui::dialog::{self, ResultAction, ResultSpec};
use gz_ui::grid::{read_only, switch, text_block, text_field};
use gz_ui::text::{self, Clip};
use gz_ui::units::em;
use gz_ui::{
    Button, Dialog, DialogSize, Font, Icon, Kind, PropFlags, PropertyGrid, TipExt, Tone, Tx, palette, tr,
};
use parking_lot::Mutex;
use zeroize::Zeroizing;

use crate::local::{local_code, local_secret_row};
use crate::{asker, open_page, reason};
use gz_ui_mod::KeyLine;

/// GitHub stops a release body at 125 000 characters; the editor takes 120 000.
pub(crate) const MAX_NOTES: usize = 120_000;
const MAX_TITLE: usize = 200;

/// Where a problem sends the author.
const FIX_SETTINGS: u8 = 1;
const FIX_RELEASES: u8 = 2;

// ---- the pure part ----------------------------------------------------------------------------

/// `v1.2.0` and the fresh tags of the same version (`v1.2.0-r2`, ...): publishing refuses a version
/// that is out under any of them.
#[must_use]
pub(crate) fn is_version_tag(tag: &str, version_tag: &str) -> bool {
    let Some(rest) = tag.strip_prefix(version_tag) else { return false };
    if rest.is_empty() {
        return true;
    }
    rest.strip_prefix("-r").is_some_and(|n| !n.is_empty() && n.bytes().all(|c| c.is_ascii_digit()))
}

/// What GitHub would refuse after the packaging, known before the click: a published release of
/// this version, or one that carries the tag.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Clash {
    pub found: bool,
    /// The version is out; otherwise only the tag is taken.
    pub version: bool,
    pub id: u64,
    pub tag: String,
}

/// Nothing while the dialog has no list of this repository's releases.
#[must_use]
pub(crate) fn published_clash(view: &ManageView, repo: &str, version: &str, tag: &str) -> Clash {
    let mut clash = Clash::default();
    if !view.listed || view.repo != repo {
        return clash;
    }
    let version_ok = gz_mod::validate_version(version);
    let version_tag = format!("v{version}");
    for r in &view.releases {
        if r.draft {
            continue;
        }
        let is_version = version_ok && is_version_tag(&r.tag, &version_tag);
        if !is_version && (clash.found || r.tag != tag) {
            continue;
        }
        clash = Clash { found: true, version: is_version, id: r.id, tag: r.tag.clone() };
        if is_version {
            break;
        }
    }
    clash
}

/// MAJOR.MINOR.PATCH, what packaging takes for `requires_game`.
#[must_use]
pub(crate) fn game_version_ok(v: &str) -> bool {
    v.bytes().filter(|&c| c == b'.').count() == 2 && gz_mod::validate_version(v)
}

/// What the problems are worked out from.
pub(crate) struct ProblemInput<'a> {
    pub repo: &'a str,
    pub version: &'a str,
    pub requires: &'a str,
    pub key_locked: bool,
    pub tag: &'a str,
    pub title: &'a str,
    pub clash: &'a Clash,
    /// A release is being deleted or edited.
    pub managing: bool,
}

/// Every reason Publish is off, in the order the dialog shows the fields, and where they are
/// fixed.
#[must_use]
pub(crate) fn problems(i: &ProblemInput<'_>) -> (Vec<String>, u8) {
    let mut out = Vec::new();
    let mut fixes = 0;
    if i.repo.is_empty() {
        out.push("No GitHub repository: set one in Mod Settings.".to_owned());
    } else if !gz_mod::validate_github_repo(i.repo) {
        out.push(format!("The GitHub repository in Mod Settings is not owner/repo: {}.", i.repo));
    }
    if !gz_mod::validate_version(i.version) {
        out.push(
            "The version in Mod Settings is not valid: 1-4 numbers separated by dots, e.g. 1.2.0.".to_owned(),
        );
    }
    if !out.is_empty() {
        fixes |= FIX_SETTINGS;
    }
    if i.clash.version {
        out.push(format!(
            "Version {} is already published as {}, and a published release is never overwritten: raise the version in Mod Settings, or delete that release in Releases first.",
            i.version, i.clash.tag
        ));
        fixes |= FIX_SETTINGS | FIX_RELEASES;
    }
    if !i.requires.is_empty() && !game_version_ok(i.requires) {
        out.push(format!("The linked game's version {} is not MAJOR.MINOR.PATCH.", i.requires));
    }
    if i.key_locked {
        out.push(
            "The author key is locked: give its password below - the release is signed with it.".to_owned(),
        );
    }
    if !tags::valid_tag(i.tag) {
        out.push(
            "The tag takes 1-100 letters, digits, '.', '_' and '-', and starts with neither '-' nor '.'."
                .to_owned(),
        );
    } else if i.clash.found && !i.clash.version {
        out.push(format!(
            "Tag {} is already published, and a published release is never overwritten: pick another tag.",
            i.tag
        ));
    }
    if i.title.len() > MAX_TITLE {
        out.push("The title is longer than 200 characters.".to_owned());
    }
    if i.managing {
        out.push("A release is being deleted or edited: wait for it to finish.".to_owned());
    }
    (out, fixes)
}

// ---- what the pools leave for the frames ------------------------------------------------------

/// Read off the UI thread when the dialog opens, and again after an action that changes them.
#[derive(Default)]
struct Facts {
    /// Where the token comes from and the variable that supplied it; `None` while it is read.
    account: Option<(AccountSource, String)>,
    account_error: String,
    /// The linked game's version; `None` while it is read.
    game_version: Option<String>,
    /// `releases\deleted.tags`; `None` while it is read.
    deleted_tags: Option<String>,
    /// An earlier packaging of a version, by file.
    replaced: Option<(String, Vec<String>)>,
    device_sign_in: bool,
    /// GitHub's web address (the QA mock's under `-qa_github`).
    web: String,
    /// A read of the list that did not start: the repository and why.
    list_error: Option<(String, String)>,
    /// A publish the dialog asked for that did not start.
    start_error: Option<String>,
    /// The author key wants its password, as read when the dialog opened; `None` while it is read.
    key_locked: Option<bool>,
}

/// What the dialog names of the module, from the manifest store's snapshot.
#[derive(Clone, Debug, Default)]
struct Module {
    id: String,
    name: String,
    repo: String,
    version: String,
}

fn module_of(editor: &Editor) -> Option<Module> {
    let session = editor.project_opt()?;
    let svc = ManifestService::get(&session)?;
    let snap = svc.snapshot();
    let m = snap.manifest()?;
    Some(Module {
        id: m.id.clone(),
        name: m.name.clone(),
        repo: m.update_github.clone(),
        version: m.version.clone(),
    })
}

/// A publish started here, followed to its end for the result dialog.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Watch {
    #[default]
    None,
    Starting,
    Running,
}

/// The device sign-in as `sign_in_status` last said.
#[derive(Clone, Debug, Default)]
struct Sign {
    state: String,
    code: String,
    uri: String,
    error: String,
}

/// What a click of the dialog asks for, done once it is drawn.
enum Act {
    Settings,
    Releases,
    Publish,
    SignIn,
    SignStop,
    ForgetToken,
    SaveToken,
    Copy(String),
    Page(String),
    RepoPage(String),
}

// ---- the Publish Release dialog ---------------------------------------------------------------

pub(crate) struct PublishDialog {
    open: bool,
    was_open: bool,
    notes: String,
    tag: String,
    /// The tag follows the version until the author edits it.
    tag_auto: bool,
    tag_for: String,
    title: String,
    title_auto: bool,
    prerelease: bool,
    latest: bool,
    token: Zeroizing<String>,
    key: KeyLine,
    /// The row showed the key locked since the dialog opened: once it no longer does, the
    /// password was given.
    key_seen_locked: bool,
    facts: Arc<Mutex<Facts>>,
    /// The repository and account the open dialog read the releases of.
    list_asked_for: String,
    replaced_for: String,
    queued: Arc<AtomicBool>,
    watch: Watch,
    signing: bool,
    sign: Sign,
    /// The code was copied and GitHub's page opened.
    sign_shown: bool,
}

impl Default for PublishDialog {
    fn default() -> Self {
        Self {
            open: false,
            was_open: false,
            notes: String::new(),
            tag: String::new(),
            tag_auto: true,
            tag_for: String::new(),
            title: String::new(),
            title_auto: true,
            prerelease: false,
            latest: true,
            token: Zeroizing::new(String::with_capacity(300)),
            key: KeyLine::default(),
            key_seen_locked: false,
            facts: Arc::new(Mutex::new(Facts::default())),
            list_asked_for: String::new(),
            replaced_for: String::new(),
            queued: Arc::new(AtomicBool::new(false)),
            watch: Watch::None,
            signing: false,
            sign: Sign::default(),
            sign_shown: false,
        }
    }
}

/// Reads where the token comes from again; the token itself goes at once.
fn refresh_account(editor: &Arc<Editor>, facts: &Arc<Mutex<Facts>>) {
    facts.lock().account = None;
    let (ed, facts) = (editor.clone(), facts.clone());
    editor.exec.spawn(async move {
        let account = Github::of(&ed).resolve_account().await;
        let read = (account.source, account.detail.clone());
        drop(account);
        facts.lock().account = Some(read);
        ed.exec.wake().ping();
    });
}

impl PublishDialog {
    /// Opens the dialog: the fields start over and what it shows is read off the UI thread.
    fn open_now(&mut self, editor: &Arc<Editor>) -> Result<(), String> {
        let session = editor.project().map_err(|e| e.message)?;
        if let Some(svc) = ManifestService::get(&session)
            && !svc.snapshot().exists()
        {
            return Err("Cannot publish the release: mod.ltx is not in the project.".to_owned());
        }
        self.notes.clear();
        self.tag.clear();
        self.tag_auto = true;
        self.tag_for.clear();
        self.title_auto = true;
        self.prerelease = false;
        self.latest = true;
        self.replaced_for.clear();
        self.list_asked_for.clear();
        self.wipe();
        self.facts = Arc::new(Mutex::new(Facts::default()));
        self.key.refresh(editor);
        refresh_account(editor, &self.facts);

        let (ed, facts) = (editor.clone(), self.facts.clone());
        editor.exec.spawn(async move {
            // the store follows mod.ltx from here on: the frames read its snapshot
            if let Ok(m) = ManifestService::of_session(&ed, &session).await {
                let _ = m.sync().await;
            }
            let game =
                session.services.get::<gz_project::Project>().map(|p| p.game_root()).unwrap_or_default();
            let game_version = ed.exec.io(move || gz_release::game_version::linked_game_version(&game)).await;
            let file = tags::deleted_tags_file(&session.root);
            let deleted = ed.exec.io(move || std::fs::read_to_string(file).unwrap_or_default()).await;
            let key = gz_release::key_state(&ed).await;
            let gh = Github::of(&ed);
            let mut f = facts.lock();
            f.key_locked = Some(key.exists && key.error.is_empty() && key.locked);
            f.game_version = Some(game_version);
            f.deleted_tags = Some(deleted);
            f.device_sign_in = gh.endpoints().device_sign_in();
            f.web = gh.endpoints().web.clone();
            drop(f);
            ed.exec.wake().ping();
        });
        self.open = true;
        Ok(())
    }

    /// A sign-in nobody is watching and a token nobody is going to save.
    fn wipe(&mut self) {
        self.token = Zeroizing::new(String::with_capacity(300));
        // a password half typed into the key row goes with it
        self.key = KeyLine::default();
        self.key_seen_locked = false;
    }

    fn cleanup(&mut self, editor: &Arc<Editor>) {
        if self.signing {
            Publisher::of(editor).sign_stop();
        }
        self.signing = false;
        self.sign = Sign::default();
        self.wipe();
    }

    /// Reads the repository's releases once the dialog is up, and again for another repository
    /// or account.
    fn ensure_list(
        &mut self,
        editor: &Arc<Editor>,
        repo: &str,
        account: Option<AccountSource>,
        view: &ManageView,
    ) {
        let Some(source) = account else { return };
        if source == AccountSource::None || !gz_mod::validate_github_repo(repo) {
            return;
        }
        let key = format!("{repo}|{}", source.text());
        if key == self.list_asked_for || Publisher::of(editor).running() {
            return;
        }
        if view.state == ManageState::Running {
            // a list of this repository is on its way already
            if view.op == ManageOp::List && view.repo == repo {
                self.list_asked_for = key;
            }
            return;
        }
        self.list_asked_for = key;
        self.facts.lock().list_error = None;
        let (ed, facts, repo) = (editor.clone(), self.facts.clone(), repo.to_owned());
        editor.exec.spawn(async move {
            if let Err((e, _)) = ReleaseList::of(&ed).start(&ed, ManageRequest::default()).await {
                facts.lock().list_error = Some((repo, e));
            }
            ed.exec.wake().ping();
        });
    }

    /// The earlier packaging of this version, read when the version changes.
    fn ensure_replaced(&mut self, editor: &Arc<Editor>, module: &Module) {
        if self.replaced_for == module.version {
            return;
        }
        self.replaced_for.clone_from(&module.version);
        let Some(root) = editor.project_root() else { return };
        let (ed, facts) = (editor.clone(), self.facts.clone());
        let (id, version) = (module.id.clone(), module.version.clone());
        editor.exec.spawn(async move {
            let v = version.clone();
            let files = ed.exec.io(move || gz_release::packaged_files(&root, "", &id, &v)).await;
            facts.lock().replaced = Some((version, files));
            ed.exec.wake().ping();
        });
    }

    /// The device sign-in as it stands, while one runs from here.
    fn poll_sign_in(&mut self, ctx: &egui::Context, editor: &Arc<Editor>, web: &str) {
        if !self.signing {
            return;
        }
        let v = Publisher::of(editor).sign_view();
        let waiting = v.state == SignState::Waiting;
        self.sign = Sign {
            state: match v.state {
                SignState::Idle => "idle",
                SignState::Starting => "starting",
                SignState::Waiting => "waiting",
                SignState::Done => "done",
                SignState::Failed => "failed",
            }
            .to_owned(),
            code: if waiting { v.user_code } else { String::new() },
            uri: if waiting { v.uri } else { String::new() },
            error: v.error,
        };
        match self.sign.state.as_str() {
            "done" => {
                self.signing = false;
                refresh_account(editor, &self.facts);
            }
            "failed" | "idle" => self.signing = false,
            "waiting" if !self.sign_shown => {
                self.sign_shown = true;
                ctx.copy_text(self.sign.code.clone());
                open_page(ctx, &self.sign.uri, web);
            }
            _ => {}
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }

    fn draw(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>, hub_refresh: &mut bool) {
        let editor = cx.editor.clone();
        if self.was_open && !self.open {
            self.cleanup(&editor);
        }
        self.was_open = self.open;
        self.follow_job(ui.ctx(), &editor, hub_refresh);
        if !self.open {
            return;
        }
        if editor.project_opt().is_none() {
            self.open = false;
            return;
        }
        let module = module_of(&editor).unwrap_or_default();
        let (account, account_error, game_version, deleted, replaced, device, web, list_error, read_locked) = {
            let f = self.facts.lock();
            (
                f.account.clone(),
                f.account_error.clone(),
                f.game_version.clone(),
                f.deleted_tags.clone(),
                f.replaced.clone(),
                f.device_sign_in,
                f.web.clone(),
                f.list_error.clone(),
                f.key_locked,
            )
        };
        if self.tag_auto
            && self.tag_for != module.version
            && let Some(deleted) = &deleted
        {
            self.tag = tags::fresh_tag(&module.version, deleted, tags::now_seconds());
            self.tag_for.clone_from(&module.version);
        }
        if self.title_auto {
            let title = tags::default_title(&module.id, &module.name, &module.version);
            if title != self.title {
                self.title = title;
            }
        }
        self.ensure_replaced(&editor, &module);
        self.poll_sign_in(ui.ctx(), &editor, &web);

        let list = ReleaseList::of(&editor);
        let view = list.view();
        self.ensure_list(&editor, &module.repo, account.as_ref().map(|a| a.0), &view);
        let reading_list = view.state == ManageState::Running && view.op == ManageOp::List;
        let reading = reading_list
            || account.is_none()
            || game_version.is_none()
            || read_locked.is_none()
            || (self.tag_auto && self.tag_for != module.version);
        let list_failed = if view.state == ManageState::Failed
            && view.op == ManageOp::List
            && view.repo == module.repo
            && !view.error.is_empty()
        {
            Some(view.error.clone())
        } else {
            list_error.filter(|(repo, _)| *repo == module.repo).map(|(_, e)| e)
        };
        let clash = published_clash(&view, &module.repo, &module.version, &self.tag);
        let requires = game_version.clone().unwrap_or_default();
        let source = account.as_ref().map_or(AccountSource::None, |a| a.0);
        // the row says whether the key is locked once it has drawn; until then, the read of the opening
        if self.key.wants_password() {
            self.key_seen_locked = true;
        }
        let key_locked =
            read_locked.unwrap_or(false) && !(self.key_seen_locked && !self.key.wants_password());
        let (problems, fixes) = problems(&ProblemInput {
            repo: &module.repo,
            version: &module.version,
            requires: &requires,
            key_locked,
            tag: &self.tag,
            title: &self.title,
            clash: &clash,
            managing: view.state == ManageState::Running && view.op != ManageOp::List,
        });
        let replaced =
            replaced.filter(|(v, _)| *v == module.version).map(|(_, files)| files).unwrap_or_default();

        let mut acts: Vec<Act> = Vec::new();
        let mut open = self.open;
        let ctx = ui.ctx().clone();
        let status = if reading || !problems.is_empty() {
            ""
        } else {
            "Packages, uploads a draft, checks every file, then publishes."
        };
        // not signed in is said once, in the Account row beside its action; it still stops Publish
        let signed_out = account.is_some() && source == AccountSource::None;
        let can = problems.is_empty() && !signed_out && !reading && !self.queued.load(Ordering::Relaxed);
        let why = problems.first().map_or_else(
            || {
                Tx::from(if signed_out {
                    "Not signed in to GitHub: sign in or save a token in the Account row."
                } else if reading {
                    "Reading the releases on GitHub first."
                } else {
                    "Publishing is starting."
                })
            },
            |p| Tx::ui(p.clone()),
        );
        let mut publish = false;
        let mut cancel = false;
        Dialog::new("Publish Release", "dlg.publish_release", DialogSize::L).show(
            ui,
            &mut open,
            |ui| {
                for problem in &problems {
                    text::status(ui, Tone::Warning, Tx::ui(problem.as_str()));
                }
                // where each problem is fixed, by name: Repository and Version are read-only here
                if fixes != 0 {
                    ui.horizontal(|ui| {
                        if fixes & FIX_SETTINGS != 0
                            && Button::new("Open Mod Settings").tip("Closes this dialog and opens Mod Settings, where the repository and the version are set.").show(ui).clicked()
                        {
                            acts.push(Act::Settings);
                        }
                        if fixes & FIX_RELEASES != 0
                            && Button::new("Open Releases").tip("Closes this dialog and opens the Releases tab on the release of this version.").show(ui).clicked()
                        {
                            acts.push(Act::Releases);
                        }
                    });
                }
                if let Some(e) = &list_failed {
                    text::status(ui, Tone::Muted, tr!("The releases of {repo} could not be read: {why}. Publishing checks the version on GitHub again.", repo = &module.repo, why = Tx::ui(reason(e).to_owned())));
                }
                if !problems.is_empty() || list_failed.is_some() {
                    ui.add_space(4.0);
                }
                self.grid(ui, &editor, &module, &clash, source, account.as_ref(), &account_error, device, &requires, key_locked, &mut acts);
                if !replaced.is_empty() {
                    ui.add_space(4.0);
                    let n = replaced.len();
                    text::status(
                        ui,
                        Tone::Warning,
                        tr!("Publishing replaces {n} {n:file|files} of an earlier packaging in releases\\{version}:", n = n, version = &module.version),
                    );
                    let color = palette(ui.ctx()).text_secondary;
                    ui.indent("replaced", |ui| {
                        for name in replaced.iter().take(10) {
                            text::label(ui, Tx::raw(name.as_str()), Font::Body, color, Clip::Middle);
                        }
                        if n > 10 {
                            text::label(ui, tr!("and {n} more", n = n - 10), Font::Body, color, Clip::End);
                        }
                    });
                }
            },
            |f| {
                f.status(status, Tone::Muted);
                // the releases that are out decide whether this version may go: nothing is offered before they are known
                if reading {
                    f.busy("Reading the releases on GitHub...");
                }
                if f.button("Publish", Kind::Primary, false, can, why) {
                    publish = true;
                }
                if f.button("Cancel", Kind::Secondary, false, true, "") {
                    cancel = true;
                }
            },
        );
        if publish {
            acts.push(Act::Publish);
        }
        self.open = open && !cancel;
        for act in acts {
            self.run(act, cx, &editor, &ctx, &clash, &web, game_version.as_deref(), hub_refresh);
        }
    }

    /// The property grid: the Release line and the Notes, then what has a good default under More.
    #[allow(clippy::too_many_arguments)]
    fn grid(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        module: &Module,
        clash: &Clash,
        source: AccountSource,
        account: Option<&(AccountSource, String)>,
        account_error: &str,
        device: bool,
        requires: &str,
        key_locked: bool,
        acts: &mut Vec<Act>,
    ) {
        let repo_ok = !module.repo.is_empty() && gz_mod::validate_github_repo(&module.repo);
        let version_ok = gz_mod::validate_version(&module.version) && !clash.version;
        let signed_in = source != AccountSource::None || account.is_none();
        let tag_wrong =
            !tags::valid_tag(&self.tag) || (clash.found && !clash.version) || self.title.len() > MAX_TITLE;
        // a label column in em: the labels are a word or two, the values take the rest
        // the labels' column follows the longest of them in the window's language
        let label_w = [
            "Release",
            "Notes",
            "Tag",
            "Title",
            "Options",
            "Requires Game",
            "Author Key",
            "Account",
            "Token",
        ]
        .iter()
        .map(|l| text::width(ui.ctx(), &Font::Body.id(), *l) + em(1.0))
        .fold(em(9.0), f32::max);
        let frac = label_w / ui.available_width().max(1.0);
        PropertyGrid::new("publish").label_frac(frac).show(ui, |g| {
            // what goes out in one line, what to say about it, and everything with a good default under More
            let flags = if repo_ok && version_ok { PropFlags::READ_ONLY } else { PropFlags::READ_ONLY | PropFlags::INVALID };
            let name = if module.name.is_empty() { &module.id } else { &module.name };
            let line = tr!(
                "publish" => "{name}  ·  {version}  ·  {repo}",
                name = name.as_str(),
                version = if module.version.is_empty() { Tx::from("no version") } else { Tx::raw(module.version.clone()) },
                repo = if module.repo.is_empty() { Tx::from("no repository") } else { Tx::raw(module.repo.clone()) }
            );
            g.row("Release", "What goes out and where players update from. Mod Settings sets them.", flags, 0, |ui| read_only(ui, line, Clip::End));
            if repo_ok && g.trailing(Icon::External, "Open the repository on GitHub") {
                acts.push(Act::RepoPage(module.repo.clone()));
            }
            g.block(|ui| {
                let p = palette(ui.ctx());
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    let _ = text::label(ui, "Notes", Font::Body, p.text_secondary, Clip::End)
                        .tip("What changed, shown on the release page. Markdown.");
                });
                let w = ui.available_width().max(1.0);
                let hint = gz_ui::i18n::t(ui.ctx(), "What changed in this release - players read it on the release page. Markdown.");
                let resp = ui.add(
                    egui::TextEdit::multiline(&mut self.notes)
                        .font(Font::Body.id())
                        .margin(egui::Margin::symmetric(8, 6))
                        .hint_text(hint) // ui-text: translated by i18n::t above
                        .desired_rows(5)
                        .desired_width(w),
                );
                // an input reads as one: its outline, the focus ring while it has the keys
                let stroke = if resp.has_focus() {
                    egui::Stroke::new(2.0, p.accent_bright)
                } else if resp.hovered() {
                    egui::Stroke::new(1.0, p.border)
                } else {
                    egui::Stroke::new(1.0, p.border_subtle)
                };
                ui.painter().rect_stroke(resp.rect, egui::CornerRadius::same(4), stroke, egui::StrokeKind::Inside);
                gz_ui::audit::control(ui.ctx(), "Notes", true, None, Some("What changed, shown on the release page. Markdown."), resp.rect);
                ui.add_space(6.0);
            });
            // what has to be dealt with before this can go is never folded away
            if key_locked {
                self.key_row(g, editor, true);
            }
            if !signed_in {
                self.account_rows(g, source, account, account_error, device, acts);
            }
            g.section("More", tag_wrong, None, |g| {
                let tag_flags = if tags::valid_tag(&self.tag) && !(clash.found && !clash.version) { PropFlags::NONE } else { PropFlags::INVALID };
                let help = "The git tag GitHub creates for the release; the game does not read it. A tag deleted within the last hour is not offered again: GitHub keeps serving a deleted release's files under a reused tag for minutes.";
                if g.row("Tag", help, tag_flags, 0, |ui| text_field(ui, &mut self.tag).changed()) == Some(true) {
                    self.tag_auto = false;
                }
                let title_flags = if self.title.len() > MAX_TITLE { PropFlags::INVALID } else { PropFlags::NONE };
                if g.row("Title", "The release's title on its GitHub page.", title_flags, 0, |ui| text_field(ui, &mut self.title).changed()) == Some(true) {
                    self.title_auto = false;
                }
                g.row("Options", "Players receive only the latest release: the game reads that one alone.", PropFlags::NONE, 0, |ui| {
                    let p = palette(ui.ctx());
                    switch(ui, &mut self.prerelease).tip("Players do not receive pre-releases: the game reads the latest release alone.");
                    text::label(ui, "Pre-release", Font::Body, p.text, Clip::End);
                    ui.add_space(16.0);
                    let mut latest = self.latest && !self.prerelease;
                    let on = !self.prerelease;
                    ui.add_enabled_ui(on, |ui| {
                        if switch(ui, &mut latest).tip("The latest release is the one players update to.").changed() {
                            self.latest = latest;
                        }
                        text::label(ui, "Set as latest", Font::Body, if on { p.text } else { p.text_disabled }, Clip::End);
                    });
                });
                g.row(
                    "Requires Game",
                    "The lowest Dead Air: Refined version this release runs on: the version of the linked game. release.publish requires_game= names another one.",
                    PropFlags::READ_ONLY,
                    0,
                    |ui| read_only(ui, if requires.is_empty() { Tx::from("Any version") } else { Tx::raw(requires) }, Clip::End),
                );
                if !key_locked {
                    self.key_row(g, editor, false);
                }
                if signed_in {
                    self.account_rows(g, source, account, account_error, device, acts);
                }
            });
        });
    }

    fn key_row(&mut self, g: &mut gz_ui::grid::Grid<'_>, editor: &Arc<Editor>, locked: bool) {
        let flags = if locked { PropFlags::REQUIRED } else { PropFlags::READ_ONLY };
        let help = "The key file that signs the release: the Mod Browser lists signed releases only. Submit to Mod Browser creates one.";
        g.row("Author Key", help, flags, 0, |ui| self.key.draw(ui, editor, "publish"));
    }

    /// Where the token comes from, the device sign-in, a pasted token.
    fn account_rows(
        &mut self,
        g: &mut gz_ui::grid::Grid<'_>,
        source: AccountSource,
        account: Option<&(AccountSource, String)>,
        account_error: &str,
        device: bool,
        acts: &mut Vec<Act>,
    ) {
        let signed_in = source != AccountSource::None;
        let signing = matches!(self.sign.state.as_str(), "starting" | "waiting") && self.signing;
        let text_ = match account {
            None => Tx::EMPTY,
            Some(_) if !signed_in => Tx::from("Not signed in to GitHub: sign in or save a token"),
            Some((s, detail)) if detail.is_empty() => Tx::from(s.text()),
            Some((s, detail)) => {
                tr!("publish" => "{source} ({detail})", source = Tx::from(s.text()), detail = detail.as_str())
            }
        };
        let help = "Where the GitHub token comes from: an environment variable, the token the editor saved in the Windows Credential Manager, or the GitHub CLI (gh auth login) - in that order.";
        g.row("Account", help, if signed_in { PropFlags::READ_ONLY } else { PropFlags::REQUIRED }, 0, |ui| {
            if account.is_none() {
                gz_ui::state::busy_line(ui, "Reading the account...");
            } else {
                read_only(ui, &text_, Clip::End);
            }
        });
        if device
            && !signing
            && g.trailing(
                Icon::Link,
                "Sign in with GitHub: opens GitHub in the browser and asks for a one-time code",
            )
        {
            acts.push(Act::SignIn);
        }
        if source == AccountSource::Saved && g.trailing(Icon::Unlink, "Forget the token the editor saved") {
            acts.push(Act::ForgetToken);
        }
        if self.sign.state == "failed" && !self.sign.error.is_empty() {
            g.message(Tone::Error, Tx::ui(self.sign.error.as_str()));
        }
        if self.signing && self.sign.state == "starting" {
            g.row(
                "Sign-in",
                "GitHub hands out a one-time code that is entered on its own page.",
                PropFlags::NONE,
                0,
                |ui| {
                    gz_ui::state::busy_line(ui, "Contacting GitHub...");
                },
            );
            if g.trailing(Icon::Close, "Cancel the sign-in") {
                acts.push(Act::SignStop);
            }
        }
        if self.signing && self.sign.state == "waiting" {
            let code = self.sign.code.clone();
            g.row("Code", "Enter this code on GitHub. The token GitHub then hands out goes into the Windows Credential Manager.", PropFlags::READ_ONLY, 0, |ui| {
                local_code(ui, &code);
            });
            if g.trailing(Icon::Close, "Cancel the sign-in") {
                acts.push(Act::SignStop);
            }
            if g.trailing(Icon::External, "Open GitHub to enter the code") {
                acts.push(Act::Page(self.sign.uri.clone()));
            }
            if g.trailing(Icon::Copy, "Copy the code") {
                acts.push(Act::Copy(code));
            }
            g.message(Tone::Muted, "The code is in the clipboard. Waiting for the confirmation on GitHub...");
        }
        let help = "A personal access token: GitHub > Settings > Developer settings > Fine-grained tokens, this repository, Contents = Read and write. Kept in the Windows Credential Manager - never in the project.";
        let hint = if signed_in { "Replace with a personal access token" } else { "Personal access token" };
        let typed = !self.token.is_empty();
        if g.row("Token", help, PropFlags::NONE, 0, |ui| {
            local_secret_row(
                ui,
                "token",
                &mut self.token,
                hint.into(),
                "Save".into(),
                typed,
                "Paste a token first.".into(),
            )
        }) == Some(true)
        {
            acts.push(Act::SaveToken);
        }
        if !account_error.is_empty() {
            g.message(Tone::Error, Tx::ui(account_error));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        &mut self,
        act: Act,
        cx: &mut DocCx<'_>,
        editor: &Arc<Editor>,
        ctx: &egui::Context,
        clash: &Clash,
        web: &str,
        game_version: Option<&str>,
        hub_refresh: &mut bool,
    ) {
        match act {
            Act::Settings => {
                self.open = false;
                cx.requests.open(OpenRequest::kind("mod_settings"));
            }
            Act::Releases => {
                self.open = false;
                if clash.found {
                    ReleaseList::of(editor).select(clash.id);
                }
                *hub_refresh = true;
                cx.requests.open(OpenRequest::kind(crate::RELEASES_KIND));
            }
            Act::Publish => self.start(cx, editor, game_version),
            Act::SignIn => {
                let _job = Publisher::of(editor).sign_in(editor);
                self.signing = true;
                self.sign_shown = false;
                self.sign = Sign { state: "starting".to_owned(), ..Sign::default() };
            }
            Act::SignStop => {
                Publisher::of(editor).sign_stop();
                self.signing = false;
                self.sign = Sign::default();
            }
            Act::ForgetToken => {
                let (ed, facts) = (editor.clone(), self.facts.clone());
                editor.exec.spawn(async move {
                    Github::of(&ed).forget_token().await;
                    refresh_account(&ed, &facts);
                });
            }
            Act::SaveToken => {
                let token = Zeroizing::new(std::mem::take(&mut *self.token));
                self.token = Zeroizing::new(String::with_capacity(300));
                let (ed, facts) = (editor.clone(), self.facts.clone());
                editor.exec.spawn(async move {
                    let saved = Github::of(&ed).save_token(Secret::from(token)).await;
                    facts.lock().account_error = saved.err().unwrap_or_default();
                    refresh_account(&ed, &facts);
                });
            }
            Act::Copy(text_) => ctx.copy_text(text_),
            Act::Page(url) => open_page(ctx, &url, web),
            Act::RepoPage(repo) => {
                let base = if web.is_empty() { "https://github.com" } else { web };
                open_page(ctx, &format!("{base}/{repo}"), web);
            }
        }
    }

    /// Publish: the one path the dialog and MCP share. The dialog closes; the Task Center's card
    /// follows the job.
    fn start(&mut self, cx: &mut DocCx<'_>, editor: &Arc<Editor>, game_version: Option<&str>) {
        self.queued.store(true, Ordering::Relaxed);
        self.open = false;
        self.watch = Watch::Starting;
        let options = PublishOptions {
            tag: self.tag.clone(),
            title: self.title.clone(),
            notes: self.notes.clone(),
            // what the dialog showed: the linked game's version, or no requirement
            requires_game: Some(game_version.unwrap_or("").to_owned()),
            prerelease: self.prerelease,
            latest: self.latest,
        };
        let (ed, facts, queued) = (editor.clone(), self.facts.clone(), self.queued.clone());
        editor.exec.spawn(async move {
            if let Err(e) = Publisher::of(&ed).start(&ed, options, true).await {
                facts.lock().start_error = Some(e);
            }
            queued.store(false, Ordering::Relaxed);
            ed.exec.wake().ping();
        });
        cx.requests.task_card(true);
    }

    /// The end of a job started here: the kit's result dialog.
    fn follow_job(&mut self, ctx: &egui::Context, editor: &Arc<Editor>, hub_refresh: &mut bool) {
        let publisher = Publisher::of(editor);
        match self.watch {
            Watch::None => {}
            Watch::Starting => {
                if self.queued.load(Ordering::Relaxed) {
                    ctx.request_repaint_after(Duration::from_millis(200));
                    return;
                }
                if let Some(e) = self.facts.lock().start_error.take() {
                    self.watch = Watch::None;
                    if let Some(line) = dialog::notice(
                        ctx,
                        asker(editor),
                        Tone::Error,
                        "Publish Release",
                        tr!("Cannot publish the release: {why}.", why = Tx::ui(reason(&e).to_owned())),
                    ) {
                        editor.log.finding(&line);
                    }
                    return;
                }
                if publisher.running() {
                    self.watch = Watch::Running;
                    ctx.request_repaint_after(Duration::from_millis(500));
                } else {
                    self.watch = Watch::None;
                    self.report(ctx, editor, hub_refresh);
                }
            }
            Watch::Running => {
                if publisher.running() {
                    ctx.request_repaint_after(Duration::from_millis(500));
                } else {
                    self.watch = Watch::None;
                    self.report(ctx, editor, hub_refresh);
                }
            }
        }
    }

    fn report(&self, ctx: &egui::Context, editor: &Arc<Editor>, hub_refresh: &mut bool) {
        let job = Publisher::of(editor).view();
        let web = self.facts.lock().web.clone();
        let (tone, headline) = match job.state {
            PublishState::Done if job.served => (
                Tone::Success,
                tr!(
                    "Release {id} {version} is published. The game already offers it: players see the update in the Mods menu.",
                    id = &job.id,
                    version = &job.version
                ),
            ),
            PublishState::Done => (
                Tone::Success,
                tr!("Release {id} {version} is published.", id = &job.id, version = &job.version),
            ),
            PublishState::Cancelled if job.github => (
                Tone::Warning,
                Tx::from(
                    "Publishing cancelled. Nothing was published; the draft stays on GitHub and the next attempt reuses it.",
                ),
            ),
            PublishState::Cancelled => (
                Tone::Warning,
                Tx::from("Publishing cancelled before anything reached GitHub. Nothing was published."),
            ),
            _ if job.refused => {
                (Tone::Error, tr!("The release was not published: {why}", why = Tx::ui(job.error.clone())))
            }
            _ => (Tone::Error, tr!("Publishing failed: {why}", why = Tx::ui(job.error.clone()))),
        };
        let mut spec = ResultSpec::new("publish_result", "Publish Release", tone, headline);
        if job.state == PublishState::Done {
            // what went out is what the packaging of this publish read
            if let Some(packed) = gz_release::Releases::of(editor).last() {
                spec.notes.extend(gz_ui_mod::build::network_note(&packed.network));
            }
            spec.rows = vec![
                ("Repository".into(), Tx::raw(job.repo.clone())),
                ("Tag".into(), Tx::raw(job.tag.clone())),
                ("Release page".into(), Tx::raw(job.url.clone())),
            ];
            if crate::page_ok(&job.url, &web) {
                let (ctx2, url) = (ctx.clone(), job.url.clone());
                spec.action = Some(ResultAction {
                    label: "Open Release Page".into(),
                    run: Arc::new(move || ctx2.open_url(egui::OpenUrl::new_tab(&url))),
                });
            }
        }
        spec.warnings = job.warnings.lines().filter(|l| !l.is_empty()).map(str::to_owned).collect();
        for line in dialog::result(ctx, asker(editor), spec) {
            editor.log.finding(&line);
        }
        // a release that went out from the window shows up in the Releases tab
        *hub_refresh = true;
    }
}

// ---- the edit dialog --------------------------------------------------------------------------

#[derive(Default)]
pub(crate) struct EditDialog {
    pub(crate) open: bool,
    target: ReleaseInfo,
    title: String,
    notes: String,
}

impl EditDialog {
    pub(crate) fn open_on(&mut self, target: ReleaseInfo) {
        self.title.clone_from(&target.title);
        self.notes.clone_from(&target.notes);
        self.target = target;
        self.open = true;
    }

    fn draw(&mut self, ui: &mut Ui, editor: &Arc<Editor>, start_error: &Arc<Mutex<String>>) {
        if !self.open {
            return;
        }
        let busy = ReleaseList::of(editor).running() || Publisher::of(editor).running();
        let too_long = self.notes.len() > MAX_NOTES;
        let title_long = self.title.len() > MAX_TITLE;
        let tag =
            if self.target.tag.is_empty() { Tx::from("(no tag)") } else { Tx::raw(self.target.tag.clone()) };
        let heading = if self.target.draft {
            tr!("Draft release {tag}", tag = tag)
        } else {
            tr!("Release {tag}", tag = tag)
        };
        let why = if too_long {
            "The notes are longer than GitHub takes."
        } else if title_long {
            "The title is longer than GitHub takes."
        } else {
            "A release job is running: wait for it to finish."
        };
        let mut open = self.open;
        let (mut save, mut cancel) = (false, false);
        Dialog::new("Edit Release", "dlg.edit_release", DialogSize::M).show(
            ui,
            &mut open,
            |ui| {
                let p = palette(ui.ctx());
                text::label(ui, &heading, Font::Strong, p.text, Clip::End);
                ui.add_space(4.0);
                text::label(ui, "Title", Font::Body, p.text_secondary, Clip::End);
                text_field(ui, &mut self.title);
                ui.add_space(4.0);
                text_block(
                    ui,
                    "Notes",
                    "What changed, shown on the release page. Markdown.",
                    &mut self.notes,
                    12,
                );
                text::status(
                    ui,
                    Tone::Muted,
                    "The files, the tag and the latest mark of the release stay as they are.",
                );
                if too_long {
                    text::status(ui, Tone::Warning, "GitHub takes notes of 120 000 characters at most.");
                }
                if title_long {
                    text::status(ui, Tone::Warning, "GitHub takes a title of 200 characters at most.");
                }
            },
            |f| {
                save = f.button("Save", Kind::Primary, true, !too_long && !title_long && !busy, why);
                cancel = f.button("Cancel", Kind::Secondary, false, true, "");
            },
        );
        if save {
            let rq = ManageRequest {
                op: ManageOp::Edit,
                id: self.target.id,
                tag: String::new(),
                title: Some(self.title.clone()),
                notes: Some(self.notes.clone()),
            };
            let (ed, slot) = (editor.clone(), start_error.clone());
            editor.exec.spawn(async move {
                let started = ReleaseList::of(&ed).start(&ed, rq).await;
                *slot.lock() = started.err().map(|(e, _)| e).unwrap_or_default();
                ed.exec.wake().ping();
            });
        }
        self.open = open && !save && !cancel;
    }
}

// ---- what the release windows share -----------------------------------------------------------

/// The Publish dialog, the edit dialog and what the Releases tab shares with them. UI thread only.
#[derive(Default)]
pub(crate) struct ReleaseUi {
    pub(crate) publish: PublishDialog,
    pub(crate) edit: EditDialog,
    /// Why a request of the Releases tab, the edit dialog or a deletion did not start.
    pub(crate) start_error: Arc<Mutex<String>>,
    /// The Releases tab reads the list again when it can.
    pub(crate) refresh_wanted: bool,
    warmed: bool,
}

impl ReleaseUi {
    /// A publish is being started from the dialog.
    pub(crate) fn publish_queued(&self) -> bool {
        self.publish.queued.load(Ordering::Relaxed)
    }

    /// `ui.dialog open id=publish_release`, the menu, the toolbar: a running job brings its card
    /// back instead.
    pub(crate) fn request_publish(&mut self, cx: &mut DocCx<'_>) -> Result<(), String> {
        if self.publish_queued() {
            return Err("Publishing is starting.".to_owned());
        }
        // one modal at a time: two dialogs that open themselves close each other every frame
        if self.edit.open {
            return Err("The edit dialog of a release is up: one release dialog at a time.".to_owned());
        }
        if Publisher::of(cx.editor).running() {
            cx.requests.task_card(true);
            return Ok(());
        }
        let editor = cx.editor.clone();
        self.publish.open_now(&editor)
    }

    /// The edit dialog of a release, unless the Publish dialog is up.
    pub(crate) fn open_edit(&mut self, target: ReleaseInfo) -> bool {
        if self.publish.open {
            return false;
        }
        self.edit.open_on(target);
        true
    }

    pub(crate) fn draw(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let editor = cx.editor.clone();
        if !self.warmed {
            // the GitHub CLI is slow to answer: the first dialog that asks should find its answer
            self.warmed = true;
            Github::of(&editor).warm_account();
        }
        let list = ReleaseList::of(&editor);
        // release.view select= edit=true: the edit dialog, to be looked at
        if let Some(target) = list.take_edit_request() {
            self.open_edit(target);
        }
        if editor.project_opt().is_none() {
            self.edit.open = false;
        }
        let mut refresh = self.refresh_wanted;
        self.publish.draw(ui, cx, &mut refresh);
        self.refresh_wanted = refresh;
        self.edit.draw(ui, &editor, &self.start_error);
        list.set_publish_dialog_open(self.publish.open);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(id: u64, tag: &str, draft: bool) -> ReleaseInfo {
        ReleaseInfo { id, tag: tag.to_owned(), draft, ..ReleaseInfo::default() }
    }

    fn view(releases: Vec<ReleaseInfo>) -> ManageView {
        ManageView { listed: true, repo: "me/od".to_owned(), releases, ..ManageView::default() }
    }

    #[test]
    fn version_tags() {
        assert!(is_version_tag("v1.2.0", "v1.2.0"));
        assert!(is_version_tag("v1.2.0-r2", "v1.2.0"));
        assert!(!is_version_tag("v1.2.0-r", "v1.2.0"));
        assert!(!is_version_tag("v1.2.0-rc1", "v1.2.0"));
        assert!(!is_version_tag("v1.2.01", "v1.2.0"));
        assert!(!is_version_tag("v1.2", "v1.2.0"));
    }

    #[test]
    fn a_version_that_is_out_clashes() {
        let v = view(vec![
            release(3, "v1.3.0", true),
            release(2, "v1.2.0-r2", false),
            release(1, "v1.1.0", false),
        ]);
        let c = published_clash(&v, "me/od", "1.2.0", "v1.2.0-r3");
        assert_eq!(c, Clash { found: true, version: true, id: 2, tag: "v1.2.0-r2".to_owned() });
        // a draft of the version is no clash
        assert!(!published_clash(&v, "me/od", "1.3.0", "v1.3.0").found);
        // only the tag taken
        let c = published_clash(&v, "me/od", "2.0.0", "v1.1.0");
        assert!(c.found && !c.version && c.id == 1);
        // another repository's list says nothing
        assert!(!published_clash(&v, "me/other", "1.2.0", "v1.2.0").found);
    }

    #[test]
    fn problems_in_the_order_of_the_fields() {
        let clash = Clash { found: true, version: true, id: 2, tag: "v1.2.0".to_owned() };
        let i = ProblemInput {
            repo: "",
            version: "1.2.0",
            requires: "1.4",
            key_locked: true,
            tag: "v1.2.0",
            title: "OD 1.2.0",
            clash: &clash,
            managing: false,
        };
        let (out, fixes) = problems(&i);
        assert_eq!(fixes, FIX_SETTINGS | FIX_RELEASES);
        assert!(out[0].starts_with("No GitHub repository"));
        assert!(out[1].contains("already published as v1.2.0"));
        assert!(out[2].contains("1.4 is not MAJOR.MINOR.PATCH"));
        assert!(out[3].starts_with("The author key is locked"));
        // the account row says it, with its action beside it: not a second time at the top
        assert!(!out.iter().any(|l| l.starts_with("Not signed in")));
        let none = Clash::default();
        let ok = ProblemInput { repo: "me/od", requires: "1.4.2", key_locked: false, clash: &none, ..i };
        assert_eq!(problems(&ok), (Vec::new(), 0));
    }

    #[test]
    fn game_versions() {
        assert!(game_version_ok("1.4.2"));
        assert!(!game_version_ok("1.4"));
        assert!(!game_version_ok("1.4.2.1"));
    }
}
