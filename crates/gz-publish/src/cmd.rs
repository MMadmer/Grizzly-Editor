//! The commands: `release.publish` (package, scan a listed module, put the release on GitHub - a
//! job the command waits for), `release.account` (where the GitHub account comes from, a token
//! stored, the device sign-in), `release.list`, `release.edit` and `release.delete` (what the
//! repository holds). They ask nothing: a caller that wants a permission asks for it on its own
//! side. A token goes in and never comes out.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{CmdError, Editor, JobId, Registry, Reply, Spec};
use gz_github::{Github, Secret};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use zeroize::Zeroizing;

use crate::job::{MAX_NOTES, PublishOptions, PublishState, Publisher, reply};
use crate::manage::{ManageOp, ManageRequest, ReleaseList};

pub fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::publish(
            "release.publish",
            "Packages the module and publishes it as the latest GitHub release of [update] github; answers when it is out.",
        )
        .example("notes=\"Fixes the night shift.\""),
        release_publish,
    );
    reg.cmd(
        Spec::write(
            "release.account",
            "The GitHub account and the VirusTotal key: where they come from, a token or key stored, the device sign-in, signing out.",
        )
        .example("action=status"),
        release_account,
    );
    reg.cmd(
        Spec::read("release.list", "The releases the module's GitHub repository holds, drafts included."),
        release_list,
    );
    reg.cmd(
        Spec::write("release.edit", "Changes the title or the notes of a release on GitHub.")
            .example("tag=v1.2.0 notes=\"...\""),
        release_edit,
    );
    reg.cmd(
        Spec::destroy(
            "release.delete",
            "Deletes a release and its git tag on GitHub for good; players are then served the release before it.",
        )
        .example("tag=v1.2.0"),
        release_delete,
    );
}

fn refuse(message: impl Into<String>) -> CmdError {
    CmdError::new("refused", message)
}

const fn yes() -> bool {
    true
}

// ---- release.publish --------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PublishParams {
    /// The release notes (Markdown).
    #[serde(default)]
    notes: String,
    /// The git tag. Left out: `v<version>`, or `v<version>-rN` while a plainer one was deleted
    /// within the hour.
    #[serde(default)]
    tag: String,
    /// The title. Left out: `<name> <version>`.
    #[serde(default)]
    title: String,
    /// The lowest game version the release runs on. Left out: the linked game's version; empty:
    /// no requirement.
    #[serde(default)]
    requires_game: Option<String>,
    /// Marks the release a pre-release: it never becomes the latest.
    #[serde(default)]
    prerelease: bool,
    /// Makes it the latest release - the one players are served.
    #[serde(default = "yes")]
    latest: bool,
    /// The author key's password, when its file has one. Never echoed or logged.
    #[serde(default)]
    password: Option<String>,
    /// Wait until the release is out or refused (true), or answer the packaging's job at once
    /// (false; its result names the upload's job, `job.wait` with no job waits for both).
    #[serde(default = "yes")]
    wait: bool,
}

async fn release_publish(ed: Arc<Editor>, p: PublishParams, _origin: Origin) -> Reply {
    let publisher = Publisher::of(&ed);
    gz_release::unlock_with(&ed, p.password).await?;
    let options = PublishOptions {
        tag: p.tag,
        title: p.title,
        notes: p.notes,
        requires_game: p.requires_game,
        prerelease: p.prerelease,
        latest: p.latest,
    };
    let id = publisher.start(&ed, options, false).await.map_err(refuse)?;
    if !p.wait {
        return Ok(json!({ "job": id.0 }));
    }
    ed.jobs.wait(id).await;
    if let Some(upload) = publisher.upload_job() {
        ed.jobs.wait(upload).await;
    }
    let (ok, mut v) = publisher.started_json();
    // a state the jobs left behind them: never "still running" once both ended
    if ok
        && publisher.view().state.running()
        && let Value::Object(map) = &mut v
    {
        map.insert("state".into(), json!(PublishState::Failed.text()));
    }
    reply(ok, v)
}

// ---- release.account --------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AccountAction {
    /// Whether a GitHub account and a VirusTotal key are there, and where they come from.
    #[default]
    Status,
    /// Stores a GitHub token (`token`) in the Credential Manager.
    SaveToken,
    /// The device sign-in: answers the code to confirm on GitHub and the job that waits for it.
    SignIn,
    /// Forgets the stored GitHub token (and stops a sign-in in flight).
    SignOut,
    /// Stores a VirusTotal API key (`key`) in the Credential Manager.
    SaveVtKey,
    /// Forgets the stored VirusTotal key. The author key is never touched.
    ForgetVtKey,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AccountParams {
    #[serde(default)]
    action: AccountAction,
    /// A GitHub token, for `save_token`. Stored, never echoed or logged.
    #[serde(default)]
    token: Option<String>,
    /// A VirusTotal API key, for `save_vt_key`. Stored, never echoed or logged.
    #[serde(default)]
    key: Option<String>,
}

/// The sources alone: a token goes in and never comes out.
async fn account_json(ed: &Arc<Editor>) -> Value {
    let gh = Github::of(ed);
    let account = gh.resolve_account().await;
    let vt = gh.read_vt_key().await.is_some();
    json!({
        "signed_in": account.signed_in(),
        "source": account.source.text(),
        "device_sign_in": gh.endpoints().device_sign_in(),
        "vt_key": vt,
    })
}

async fn release_account(ed: Arc<Editor>, p: AccountParams, _origin: Origin) -> Reply {
    let publisher = Publisher::of(&ed);
    let gh = Github::of(&ed);
    let mut extra: Option<(JobId, String, String)> = None;
    match p.action {
        AccountAction::Status => {}
        AccountAction::SaveToken => {
            let Some(token) = p.token.map(Zeroizing::new).filter(|t| !t.is_empty()) else {
                return Err(CmdError::invalid("save_token needs token"));
            };
            gh.save_token(Secret::from(token)).await.map_err(refuse)?;
        }
        AccountAction::SignOut => {
            publisher.sign_stop();
            gh.forget_token().await;
        }
        AccountAction::SaveVtKey => {
            let Some(key) = p.key.map(Zeroizing::new).filter(|k| !k.is_empty()) else {
                return Err(CmdError::invalid("save_vt_key needs key"));
            };
            gh.save_vt_key(Secret::from(key)).await.map_err(refuse)?;
        }
        AccountAction::ForgetVtKey => gh.forget_vt_key().await,
        AccountAction::SignIn => {
            if !gh.endpoints().device_sign_in() {
                return Err(refuse(
                    "the device sign-in is not set up in this build: store a token (action=save_token), set GH_TOKEN or sign in to the GitHub CLI (gh auth login)",
                ));
            }
            let Some(job) = publisher.sign_in(&ed) else {
                return Err(CmdError::busy("a sign-in is under way already"));
            };
            // the caller hands the code over: no clipboard, no browser behind its back
            loop {
                let s = publisher.sign_view();
                if !s.user_code.is_empty() {
                    extra = Some((job, s.user_code, s.uri));
                    break;
                }
                if ed.jobs.get(job).is_none_or(|j| !j.running()) {
                    let info = ed.jobs.wait(job).await;
                    let why =
                        info.and_then(|i| i.error).unwrap_or_else(|| "the sign-in did not start".to_owned());
                    return Err(refuse(why));
                }
                // GitHub answers the code in a moment; the job's own deadline ends the wait
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
    let mut answer = account_json(&ed).await;
    if let (Some((job, code, uri)), Value::Object(map)) = (extra, &mut answer) {
        map.insert("job".into(), json!(job.0));
        map.insert("user_code".into(), json!(code));
        map.insert("verification_uri".into(), json!(uri));
    }
    Ok(answer)
}

// ---- release.list / edit / delete -------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListParams {
    /// Wait for GitHub's answer (true) or answer the job at once (false).
    #[serde(default = "yes")]
    wait: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditParams {
    /// The release's id, as `release.list` answers it.
    #[serde(default)]
    id: Option<u64>,
    /// Its tag, when that is what the caller knows (a draft may share a tag: then name the id).
    #[serde(default)]
    tag: Option<String>,
    /// The new title (200 characters at most). Left out: kept; empty: cleared.
    #[serde(default)]
    title: Option<String>,
    /// The new notes. Left out: kept; empty: cleared.
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DeleteParams {
    /// The release's id, as `release.list` answers it.
    #[serde(default)]
    id: Option<u64>,
    /// Its tag, when that is what the caller knows (a draft may share a tag: then name the id).
    #[serde(default)]
    tag: Option<String>,
}

/// Runs a request of the list as a job and answers what GitHub holds after it.
async fn run_list(ed: &Arc<Editor>, rq: ManageRequest, wait: bool) -> Reply {
    let list = ReleaseList::of(ed);
    // a read already on its way (the tab or the dialog asked) is the answer to wait for; a
    // request is marked running a moment before its job has an id
    let mut in_flight = list.running_job();
    for _ in 0..100 {
        if in_flight.is_some() || !list.running() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        in_flight = list.running_job();
    }
    if rq.op == ManageOp::List
        && let Some(id) = in_flight
    {
        if !wait {
            return Ok(json!({ "job": id.0 }));
        }
        ed.jobs.wait(id).await;
        let (ok, v) = list.json();
        return reply(ok, v);
    }
    let id = list.start(ed, rq).await.map_err(|(e, _)| refuse(e))?;
    if !wait {
        return Ok(json!({ "job": id.0 }));
    }
    ed.jobs.wait(id).await;
    let (ok, v) = list.json();
    reply(ok, v)
}

fn named(id: Option<u64>, tag: Option<String>) -> Result<(u64, String), CmdError> {
    let id = id.unwrap_or(0);
    let tag = tag.unwrap_or_default();
    if id == 0 && tag.is_empty() {
        return Err(CmdError::invalid("name the release: id or tag"));
    }
    Ok((id, tag))
}

async fn release_list(ed: Arc<Editor>, p: ListParams, _origin: Origin) -> Reply {
    run_list(&ed, ManageRequest { op: ManageOp::List, ..ManageRequest::default() }, p.wait).await
}

async fn release_edit(ed: Arc<Editor>, p: EditParams, _origin: Origin) -> Reply {
    let (id, tag) = named(p.id, p.tag)?;
    if p.title.is_none() && p.notes.is_none() {
        return Err(CmdError::invalid("release.edit needs title or notes"));
    }
    if p.title.as_ref().is_some_and(|t| t.len() > 200)
        || p.notes.as_ref().is_some_and(|n| n.len() > MAX_NOTES)
    {
        return Err(refuse("the title takes 200 characters, the notes 120 000"));
    }
    let rq = ManageRequest { op: ManageOp::Edit, id, tag, title: p.title, notes: p.notes };
    run_list(&ed, rq, true).await
}

async fn release_delete(ed: Arc<Editor>, p: DeleteParams, _origin: Origin) -> Reply {
    let (id, tag) = named(p.id, p.tag)?;
    let rq = ManageRequest { op: ManageOp::Delete, id, tag, ..ManageRequest::default() };
    run_list(&ed, rq, true).await
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn failed_answers_keep_their_fields() {
        let e = reply(false, json!({ "state": "failed", "error": "why" })).unwrap_err();
        assert_eq!(e.message, "why");
        assert_eq!(e.data, Some(json!({ "state": "failed" })));
        assert_eq!(reply(true, json!({ "state": "done" })).ok(), Some(json!({ "state": "done" })));
    }

    /// The doors: a deletion goes only through the destroy door, a release and a package only through
    /// the release door; the ordinary door names the right one.
    #[test]
    fn the_cards_keep_their_doors() {
        use gz_editor::{EditorOptions, Gate, Log, Registry};
        use gz_task::{ExecOptions, Runtime};
        let rt = Runtime::start(ExecOptions { async_threads: 2, cpu_threads: 1, io_threads: 1 }).unwrap();
        let (log, _guard) = Log::start(None, false).unwrap();
        let mut reg = Registry::new();
        register(&mut reg);
        gz_release::register(&mut reg);
        let ed = gz_editor::Editor::new(EditorOptions::default(), rt.exec(), log, reg);
        let call = |name: &str, a: Value, gate: Gate| {
            let args = a.as_object().cloned().unwrap_or_default();
            rt.block_on(ed.call_gated(name, args, Origin::Mcp, gate))
        };
        let code = |v: &Value| v.get("code").and_then(Value::as_str).unwrap_or("").to_owned();
        let refused = call("release.delete", json!({ "tag": "v1.0.0" }), Gate::Run);
        assert_eq!(code(&refused), "gate");
        assert!(refused["error"].as_str().unwrap().contains("grizzly_destroy"));
        assert_ne!(code(&call("release.delete", json!({ "tag": "v1.0.0" }), Gate::Destroy)), "gate");
        for name in ["release.publish", "mod.package"] {
            let refused = call(name, json!({}), Gate::Run);
            assert_eq!(code(&refused), "gate", "{name}");
            assert!(refused["error"].as_str().unwrap().contains("grizzly_release"));
            assert_eq!(code(&call(name, json!({}), Gate::Destroy)), "gate", "{name}");
            assert_ne!(code(&call(name, json!({}), Gate::Publish)), "gate", "{name}");
        }
        // the reads and the writes pass the ordinary door
        assert_ne!(code(&call("release.account", json!({}), Gate::Run)), "gate");
        assert_ne!(code(&call("release.list", json!({ "wait": false }), Gate::Run)), "gate");
        // an unknown parameter never reaches the command
        assert_eq!(code(&call("release.list", json!({ "watt": true }), Gate::Run)), "invalid");
    }

    #[test]
    fn a_release_is_named() {
        assert!(named(None, None).is_err());
        assert_eq!(named(Some(4), None).unwrap(), (4, String::new()));
    }
}
