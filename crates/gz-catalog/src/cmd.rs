//! The Mod Browser's commands, split by what they can do: `release.catalog_status` reads (the
//! requirements check, the submission as GitHub holds it, the state) and `release.catalog` reaches
//! the public (the VirusTotal scan uploads the packages, a submission or a withdrawal opens an
//! issue of the catalog). Each runs its work as a job the command waits for (`wait=false`
//! answers the job). Like every command they ask nothing. The keys are `release.key` and
//! `release.account`.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{CmdError, Editor, Registry, Reply, Spec};
use gz_project::ProjectHooks;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::service::{Catalog, Job};

pub fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::read(
            "release.catalog_status",
            "The Mod Browser: the requirements check against the latest release, the submission and listing, the state.",
        )
        .example("action=check"),
        catalog_status,
    );
    reg.cmd(
        Spec::publish(
            "release.catalog",
            "Scans the packages on VirusTotal, submits the module to the in-game Mod Browser or withdraws it.",
        )
        .example("action=submit"),
        catalog,
    );
}

/// What the area hangs on the editor: a project that closes takes the Submit window and its
/// results with it - its jobs are stopped by the job center.
pub fn install(editor: &Editor) {
    ProjectHooks::of(editor).on_close(|ed, _session| Catalog::of(ed).forget(ed));
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum StatusAction {
    /// What is known now: the keys, the account, the last job's findings, the listing.
    #[default]
    Status,
    /// Checks the requirements against the latest release, the way the game fetches it.
    Check,
    /// Reads the submission and the withdrawal as GitHub holds them, and the catalog's verdicts.
    Submission,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StatusParams {
    #[serde(default)]
    action: StatusAction,
    /// Wait for the job to end (true) or answer it at once (false).
    #[serde(default = "yes")]
    wait: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CatalogAction {
    /// Scans every package on VirusTotal with the author's own key, uploading one VirusTotal
    /// does not know.
    Scan,
    /// Checks, scans, then opens (or edits) the public submission issue of the catalog.
    Submit,
    /// Asks the catalog to take the module out of the Mod Browser.
    Withdraw,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct CatalogParams {
    action: CatalogAction,
    /// Wait for the job to end (true) or answer it at once (false).
    #[serde(default = "yes")]
    wait: bool,
}

const fn yes() -> bool {
    true
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

async fn run(ed: &Arc<Editor>, job: Option<Job>, wait: bool) -> Reply {
    if ed.project_opt().is_none() {
        return Err(CmdError::no_project());
    }
    let catalog = Catalog::of(ed);
    if let Some(job) = job {
        let id = catalog.start(ed, job, false).await.map_err(refuse)?;
        if !wait {
            return Ok(json!({ "job": id.0 }));
        }
        ed.jobs.wait(id).await;
    }
    Ok(catalog.status_json(ed).await)
}

async fn catalog_status(ed: Arc<Editor>, p: StatusParams, _origin: Origin) -> Reply {
    let job = match p.action {
        StatusAction::Status => None,
        StatusAction::Check => Some(Job::Check),
        StatusAction::Submission => Some(Job::Submission),
    };
    run(&ed, job, p.wait).await
}

async fn catalog(ed: Arc<Editor>, p: CatalogParams, _origin: Origin) -> Reply {
    // a withdrawal asks nothing here: the caller decided, and the door it came through says so
    let job = match p.action {
        CatalogAction::Scan => Job::Vt,
        CatalogAction::Submit => Job::Submit,
        CatalogAction::Withdraw => Job::Withdraw,
    };
    run(&ed, Some(job), p.wait).await
}
