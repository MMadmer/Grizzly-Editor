//! The releases that are already out (the Releases tab's data): the list, drafts included,
//! editing the title and the notes of one, deleting one together with its git tag. The work is a
//! job of the project (`releases`): a command waits for it, the Task Center can stop it, and the
//! project closing stops it. Publishing and this never run side by side.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use gz_editor::{Cancel, Editor, Executors, JobError, JobId};
use gz_github::{Github, ReleaseInfo, Secret};
use gz_mod::ManifestService;
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::job::Publisher;
use crate::tags::{deleted_tags_file, note_deleted_tag};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ManageState {
    #[default]
    Idle,
    Running,
    Done,
    Failed,
}

impl ManageState {
    fn text(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }
}

/// What a request does before it reads the list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ManageOp {
    #[default]
    List,
    Delete,
    Edit,
}

/// One release deleted or edited, or nothing - then the list.
#[derive(Clone, Debug, Default)]
pub struct ManageRequest {
    pub op: ManageOp,
    /// The release, or 0 and
    pub id: u64,
    /// its tag, when that is what the caller knows.
    pub tag: String,
    /// An edit: a field that is `None` keeps its value; an empty one clears it.
    pub title: Option<String>,
    pub notes: Option<String>,
}

/// What the list shows.
#[derive(Clone, Debug, Default)]
pub struct ManageView {
    pub state: ManageState,
    pub repo: String,
    /// What the request in flight does before it lists.
    pub op: ManageOp,
    /// Why the last request failed.
    pub error: String,
    /// ... as the tab says it: a sentence naming what failed.
    pub error_text: String,
    /// What a deletion or an edit did.
    pub note: String,
    /// The release is gone but its tag stayed.
    pub tag_kept: bool,
    /// `releases` is GitHub's answer, not an empty start.
    pub listed: bool,
    pub releases: Vec<ReleaseInfo>,
    /// Every change bumps it.
    pub generation: u64,
}

/// Why nothing can be listed at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setup {
    Ok,
    NoProject,
    NoRepo,
    NoAccount,
}

/// The releases of the open project's repository. Lives in `editor.services`.
#[derive(Default)]
pub struct ReleaseList {
    view: Mutex<ManageView>,
    /// The job of the request in flight.
    job: Mutex<Option<JobId>>,
    /// The release the tab's details show; 0 = none.
    selected: AtomicU64,
    tab_open: AtomicBool,
    publish_dialog: AtomicBool,
    /// `select show=edit`: the release whose edit dialog the UI puts up to be looked at.
    want_edit: Mutex<Option<ReleaseInfo>>,
}

/// A reason GitHub, the network or a job gave is a clause; this makes it the end of a sentence.
fn reason(text: &str) -> &str {
    text.trim_end_matches(['.', ' '])
}

impl ReleaseList {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Self> {
        editor.services.get_or_init(Self::default)
    }

    #[must_use]
    pub fn running(&self) -> bool {
        self.view.lock().state == ManageState::Running
    }

    #[must_use]
    pub fn view(&self) -> ManageView {
        self.view.lock().clone()
    }

    /// The job of the request in flight, while one runs.
    #[must_use]
    pub fn running_job(&self) -> Option<JobId> {
        if self.running() { *self.job.lock() } else { None }
    }

    /// Stops the request in flight at its next step.
    pub fn cancel(&self, editor: &Editor) {
        if self.running()
            && let Some(id) = *self.job.lock()
        {
            editor.jobs.cancel(id);
        }
    }

    /// The UI says whether the Releases tab is there.
    pub fn set_tab_open(&self, open: bool) {
        self.tab_open.store(open, Ordering::Relaxed);
    }

    /// The UI says whether the Publish Release dialog is up: one release dialog at a time.
    pub fn set_publish_dialog_open(&self, open: bool) {
        self.publish_dialog.store(open, Ordering::Relaxed);
    }

    #[must_use]
    pub fn selected(&self) -> u64 {
        self.selected.load(Ordering::Relaxed)
    }

    pub fn select(&self, id: u64) {
        self.selected.store(id, Ordering::Relaxed);
    }

    /// The edit dialog an MCP caller asked to see, once.
    pub fn take_edit_request(&self) -> Option<ReleaseInfo> {
        self.want_edit.lock().take()
    }

    /// `select`: marks the release the tab's details are about, from the list as it was last
    /// read - nothing goes to GitHub. `show_edit` asks the UI for its edit dialog, to be looked at.
    pub fn select_release(&self, id: u64, tag: &str, show_edit: bool) -> Result<(), String> {
        let view = self.view();
        let matches: Vec<&ReleaseInfo> =
            view.releases.iter().filter(|r| if id != 0 { r.id == id } else { r.tag == tag }).collect();
        if !view.listed {
            return Err("the list is not read yet: release.list first".to_owned());
        }
        let found = match matches.as_slice() {
            [] if id != 0 => return Err(format!("no release has the id {id}")),
            [] => return Err(format!("no release carries the tag {tag}")),
            [one] => (*one).clone(),
            many => return Err(format!("{} releases carry the tag {tag} - name one by its id", many.len())),
        };
        if show_edit && self.publish_dialog.load(Ordering::Relaxed) {
            return Err("the Publish Release dialog is up: one release dialog at a time".to_owned());
        }
        self.select(found.id);
        if show_edit {
            *self.want_edit.lock() = Some(found);
        }
        Ok(())
    }

    /// Starts a request: the checks, then the work as a job. Answers the job.
    pub async fn start(
        self: &Arc<Self>,
        editor: &Arc<Editor>,
        rq: ManageRequest,
    ) -> Result<JobId, (String, Setup)> {
        let Ok(session) = editor.project() else {
            return Err(("no active project".to_owned(), Setup::NoProject));
        };
        if Publisher::of(editor).running() {
            return Err(("a release is being published - wait for it to finish".to_owned(), Setup::Ok));
        }
        if self.running() {
            // a read asked for while one is on its way is that read
            if rq.op == ManageOp::List {
                for _ in 0..100 {
                    if let Some(id) = self.running_job() {
                        return Ok(id);
                    }
                    if !self.running() {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            }
            if self.running() {
                return Err(("the release list is busy - try again in a moment".to_owned(), Setup::Ok));
            }
        }
        let manifest =
            ManifestService::of_session(editor, &session).await.map_err(|e| (e.message, Setup::NoRepo))?;
        manifest.sync().await.map_err(|e| (e.message, Setup::NoRepo))?;
        let Some(repo) = manifest.snapshot().manifest().map(|m| m.update_github.clone()) else {
            return Err(("mod.ltx not found in the project".to_owned(), Setup::NoRepo));
        };
        if repo.is_empty() || !gz_mod::validate_github_repo(&repo) {
            return Err(("no GitHub repository in the manifest: set it in Mod Settings (mod.set set={\"update\":{\"github\":\"owner/repo\"}})".to_owned(), Setup::NoRepo));
        }
        let gh = Github::of(editor);
        let account = gh.resolve_account().await;
        if !account.signed_in() {
            return Err((
                "not signed in to GitHub: sign in from Mod > Publish Release, sign in to the GitHub CLI (gh auth login) or set GH_TOKEN".to_owned(),
                Setup::NoAccount,
            ));
        }

        let title = match rq.op {
            ManageOp::List => "Reading the Releases",
            ManageOp::Delete => "Deleting a Release",
            ManageOp::Edit => "Saving a Release",
        };
        let tags_file = deleted_tags_file(&session.root);
        let (me, exec) = (self.clone(), editor.exec.clone());
        let op = rq.op;
        let which = if rq.id != 0 { format!("#{}", rq.id) } else { rq.tag.clone() };
        let log = editor.log.clone();
        let repo_job = repo.clone();
        {
            let mut v = self.view.lock();
            // another repository's releases are not this one's
            if v.repo != repo {
                v.releases.clear();
                v.listed = false;
            }
            v.state = ManageState::Running;
            v.repo.clone_from(&repo);
            v.op = rq.op;
            v.error.clear();
            v.error_text.clear();
            v.note.clear();
            v.tag_kept = false;
            v.generation += 1;
        }
        // the last request's job says it is over a moment after its view did
        if let Some(last) = editor.jobs.running_in("releases") {
            editor.jobs.wait(last.id).await;
        }
        let started = editor.jobs.start("releases", Some("releases"), title, move |ctx| async move {
            ctx.stage(format!("Waiting for GitHub: {repo_job}"));
            me.work(&gh, &exec, repo_job, account.token, rq, tags_file, ctx.cancel.clone()).await;
            let v = me.view();
            if v.state == ManageState::Failed {
                if ctx.cancel.is_cancelled() {
                    return Err(JobError::from(gz_task::Cancelled));
                }
                return Err(JobError::new(v.error_text.clone()));
            }
            Ok(json!({ "releases": v.releases.len(), "note": v.note }))
        });
        let id = match started {
            Ok(id) => id,
            Err(_) => {
                self.view.lock().state = ManageState::Idle;
                return Err(("the release list is busy - try again in a moment".to_owned(), Setup::Ok));
            }
        };
        *self.job.lock() = Some(id);
        if op != ManageOp::List {
            let verb = if op == ManageOp::Delete { "deleting" } else { "editing" };
            log.info(format!("[XMS] {verb} release {which} in {repo}"));
        }
        Ok(id)
    }

    #[allow(clippy::too_many_arguments)]
    async fn work(
        &self,
        gh: &Github,
        exec: &Executors,
        repo: String,
        token: Secret,
        mut rq: ManageRequest,
        tags_file: PathBuf,
        cancel: Cancel,
    ) {
        let mut error = String::new();
        let mut note = String::new();
        let mut tag_kept = false;

        if rq.op != ManageOp::List && rq.id == 0 {
            match gh.list_releases(&repo, &token, &cancel).await {
                Ok(releases) => {
                    // a tag names one published release, but any number of drafts may share it
                    let matches: Vec<u64> =
                        releases.iter().filter(|r| r.tag == rq.tag).map(|r| r.id).collect();
                    match matches.as_slice() {
                        [] => error = format!("no release carries the tag {}", rq.tag),
                        [one] => rq.id = *one,
                        many => {
                            error = format!(
                                "{} releases carry the tag {} - name one by its id",
                                many.len(),
                                rq.tag
                            )
                        }
                    }
                }
                Err(e) => error = e,
            }
        }
        if error.is_empty() && rq.id != 0 && rq.op == ManageOp::Delete {
            match gh.delete_release(&repo, &token, rq.id, &cancel).await {
                Ok(result) => {
                    if !result.draft {
                        let (file, tag) = (tags_file.clone(), result.tag.clone());
                        exec.io(move || note_deleted_tag(&file, &tag)).await;
                    }
                    note = if result.draft {
                        format!("Draft release {} is deleted.", result.tag)
                    } else if result.tag_removed {
                        format!("Release {} and its tag are deleted.", result.tag)
                    } else {
                        tag_kept = true;
                        format!("Release {} is deleted, its tag is not: {}", result.tag, result.tag_note)
                    };
                }
                Err(e) => error = e,
            }
        }
        if error.is_empty() && rq.id != 0 && rq.op == ManageOp::Edit {
            match gh
                .edit_release(&repo, &token, rq.id, rq.title.as_deref(), rq.notes.as_deref(), &cancel)
                .await
            {
                Ok(tag) => note = format!("Release {tag} is updated."),
                Err(e) => error = e,
            }
        }
        let mut error_text = String::new();
        if !error.is_empty() {
            let what = if rq.op == ManageOp::Delete {
                "The release could not be deleted: "
            } else {
                "The release could not be saved: "
            };
            error_text = format!("{what}{}.", reason(&error));
        }

        // whatever happened above, the tab shows what GitHub has now
        let listed = gh.list_releases(&repo, &token, &cancel).await;
        drop(token);
        let mut v = self.view.lock();
        let releases = match listed {
            Ok(releases) => Some(releases),
            Err(list_error) => {
                if error.is_empty() {
                    error_text = format!("The releases could not be read: {}.", reason(&list_error));
                    error = list_error;
                }
                None
            }
        };
        v.state = if error.is_empty() { ManageState::Done } else { ManageState::Failed };
        v.error = error;
        v.error_text = error_text;
        v.note = note;
        v.tag_kept = tag_kept;
        if let Some(releases) = releases {
            v.listed = true;
            v.releases = releases;
        }
        v.generation += 1;
    }

    /// The list's answer; `ok` is false when the last request failed.
    #[must_use]
    pub fn json(&self) -> (bool, Value) {
        let v = self.view();
        let releases: Vec<Value> = v
            .releases
            .iter()
            .map(|r| {
                json!({
                    "id": r.id,
                    "tag": r.tag,
                    "title": r.title,
                    "notes": r.notes,
                    "draft": r.draft,
                    "prerelease": r.prerelease,
                    "latest": r.latest,
                    "published": r.published,
                    "url": r.url,
                    "bytes": r.bytes,
                    "assets": r.assets,
                })
            })
            .collect();
        (
            v.state != ManageState::Failed,
            json!({
                "state": v.state.text(),
                "repo": v.repo,
                "note": v.note,
                "tag_kept": v.tag_kept,
                "error": v.error,
                // what the Releases tab shows: whether it is there, and the release its details are about
                "tab": self.tab_open.load(Ordering::Relaxed),
                "selected": self.selected(),
                "listed": v.listed,
                "releases": releases,
            }),
        )
    }
}
