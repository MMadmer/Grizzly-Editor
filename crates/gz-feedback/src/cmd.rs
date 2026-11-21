//! `feedback.*`: a bug report or a suggestion to the editor's author, with an anonymous diagnostic
//! package and a crash of the editor (docs/REPORTS.md, docs/COMMANDS.md).

use std::sync::Arc;

use gz_editor::{CmdError, Editor, JobError, JobState, NoParams, Origin, Registry, Reply, Spec, Value, json};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::crashes::{self, Crash};
use crate::report::{self, Ask, Kind};
use crate::send::{self, Form, Target};
use crate::service::FeedbackService;

const GROUP: &str = "feedback";

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SendParams {
    /// `bug` (something does not work as it should, the default) or `suggestion` (something the
    /// author would like the editor to do).
    #[serde(default)]
    kind: Kind,
    /// What the report is about, in a line: 5 to 200 characters.
    title: String,
    /// What happened and how to get there again, or what is wished for and why: 20 to 10000
    /// characters, sent as written.
    text: String,
    /// Attach the anonymous diagnostic package: the editor's state, the tail of its log, its settings
    /// and the crash's files, with every name of the person and the computer taken out. Left out: on
    /// for a bug and a crash, off for a suggestion.
    #[serde(default)]
    diagnostics: Option<bool>,
    /// A crash of the editor to send with the report: its name from `feedback.status`, or `latest`.
    #[serde(default)]
    crash: Option<String>,
    /// Make the package and answer what it holds, sending nothing.
    #[serde(default)]
    dry_run: bool,
    /// Answer when the report is sent (the default); false answers `{job}` at once.
    #[serde(default = "yes")]
    wait: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DismissParams {
    /// The crash not to offer again: its name from `feedback.status`, or `latest`.
    crash: String,
}

fn service(editor: &Editor) -> Result<Arc<FeedbackService>, CmdError> {
    FeedbackService::of(editor)
        .ok_or_else(|| CmdError::new("refused", "Feedback is not running in this editor."))
}

fn crash_json(c: &Crash) -> Value {
    json!({
        "name": c.name,
        "time": c.time,
        "fatal": c.fatal,
        "handled": c.handled,
        "summary": c.summary(),
        "dump": c.dump.is_some(),
        "log": c.log.is_some(),
    })
}

/// The crash `name` names: one of the folder's, or for `latest` the newest that ended the editor (the
/// one a start offers). Reads the log folder on the IO pool.
async fn find_crash(editor: &Arc<Editor>, name: &str) -> Result<Crash, CmdError> {
    let (dir, wanted) = (editor.paths.logs.clone(), name.to_owned());
    let (all, found) = editor
        .exec
        .io(move || {
            let all = crashes::list(&dir);
            let found = if wanted == "latest" {
                crashes::latest(&dir)
            } else {
                all.iter().find(|c| c.name == wanted).cloned()
            };
            (all, found)
        })
        .await;
    found.ok_or_else(|| {
        let text = if all.is_empty() {
            "The editor has no crash to send: none is left in its log folder.".to_owned()
        } else {
            format!("No crash is named '{name}': feedback.status lists them.")
        };
        CmdError::new("not_found", text)
    })
}

/// What `feedback.status` answers; the log folder is read on the IO pool.
pub async fn status(editor: &Arc<Editor>) -> Value {
    let svc = FeedbackService::of(editor);
    let dir = editor.paths.logs.clone();
    let all = editor.exec.io(move || crashes::list(&dir)).await;
    json!({
        "available": Target::available(editor.flags.qa_github),
        "offer": svc.as_ref().and_then(|s| s.offer()).map(|o| crash_json(&o.crash)),
        "last": svc.as_ref().and_then(|s| s.last()),
        "crashes": all.iter().map(crash_json).collect::<Vec<_>>(),
    })
}

fn check_text(p: &SendParams) -> Result<(), CmdError> {
    let title = p.title.trim().chars().count();
    if !(5..=200).contains(&title) {
        return Err(CmdError::new(
            "refused",
            format!("A title is 5 to 200 characters; this one is {title}."),
        ));
    }
    let text = p.text.trim().chars().count();
    if !(20..=10_000).contains(&text) {
        return Err(CmdError::new(
            "refused",
            format!(
                "The text is 20 to 10000 characters - what happened and how to get there again; this one is {text}."
            ),
        ));
    }
    Ok(())
}

async fn send_cmd(ed: Arc<Editor>, p: SendParams, _origin: Origin) -> Reply {
    let svc = service(&ed)?;
    check_text(&p)?;
    let crash = match p.crash.as_deref() {
        Some(name) => Some(find_crash(&ed, name).await?),
        None => None,
    };
    let diagnostics = p.diagnostics.unwrap_or(crash.is_some() || p.kind == Kind::Bug);
    if crash.is_some() && !diagnostics {
        return Err(CmdError::new(
            "refused",
            "A crash goes with its diagnostics: send it with diagnostics=true, or leave the crash out.",
        ));
    }
    if !p.dry_run && !Target::available(ed.flags.qa_github) {
        return Err(CmdError::new(
            "refused",
            "This build of the editor carries no key of the report hub, so it cannot send feedback. A released build can.",
        ));
    }
    let ask = Ask {
        kind: p.kind,
        title: p.title.trim().to_owned(),
        text: p.text.trim().to_owned(),
        diagnostics,
        crash,
    };
    let dry_run = p.dry_run;
    let title = if dry_run { "Preparing Feedback" } else { "Sending Feedback" };
    let (job_ed, job_svc) = (ed.clone(), svc.clone());
    let started = ed.jobs.start_for_editor("feedback_send", Some(GROUP), title, move |ctx| async move {
        let ed = job_ed;
        let pkg = report::build(&ed, &job_svc, &ask, &ctx).await.map_err(JobError::new)?;
        let bytes = pkg.zip.as_ref().map_or(0, Vec::len);
        let files = pkg.files.iter().map(|(n, b)| json!({ "name": n, "bytes": b })).collect::<Vec<_>>();
        let crash_name = ask.crash.as_ref().map(|c| c.name.clone());
        if dry_run {
            return Ok(json!({
                "sent": false,
                "report_id": pkg.report_id,
                "bytes": bytes,
                "files": files,
                "left_out": pkg.left_out,
                "report": pkg.manifest,
                "crash": crash_name,
            }));
        }
        let target = Target::of(ed.flags.qa_github)
            .ok_or_else(|| JobError::new("This build of the editor carries no key of the report hub."))?;
        ctx.stage("Sending the report");
        let form = Form {
            title: &ask.title,
            text: &ask.text,
            kind: ask.kind.word(),
            report_id: &pkg.report_id,
            zip: pkg.zip.as_deref(),
        };
        let taken = send::send(&target, &form, &ctx.cancel).await.map_err(|e| {
            ed.log.warn(format!("[feedback] report {} was not sent: {e}", pkg.report_id));
            JobError::new(e)
        })?;
        if let Some(name) = &crash_name {
            let (dir, handled) = (ed.paths.logs.clone(), name.clone());
            let _ = ctx.exec.io(move || crashes::mark_handled(&dir, &handled)).await;
            job_svc.settle_offer(&ed, name);
        }
        ed.log.info(format!(
            "[feedback] {} {} sent ({} bytes){}",
            ask.kind.word(),
            pkg.report_id,
            bytes,
            if taken.duplicate { ", the hub had it already" } else { "" }
        ));
        let answer = json!({
            "sent": true,
            "report_id": pkg.report_id,
            "id": taken.id,
            "duplicate": taken.duplicate,
            "bytes": bytes,
            "files": files,
            "left_out": pkg.left_out,
            "crash": crash_name,
        });
        job_svc.set_last(&ed, answer.clone());
        Ok(answer)
    });
    let id = started.map_err(|busy| {
        CmdError::new("busy", format!("{} is still running.", busy.title))
            .with_data(json!({ "job": busy.id }))
    })?;
    if !p.wait {
        return Ok(json!({ "job": id }));
    }
    let info = ed.jobs.wait(id).await.ok_or_else(|| CmdError::internal("the feedback job was forgotten"))?;
    match info.state {
        JobState::Cancelled => Err(CmdError::new("stopped", "The report was not sent: it was stopped.")),
        JobState::Failed => {
            Err(CmdError::new("failed", info.error.unwrap_or_else(|| "The report was not sent.".to_owned())))
        }
        _ => Ok(info.result.map_or(Value::Null, |r| (*r).clone())),
    }
}

/// Registers `feedback.status`, `feedback.send` and `feedback.dismiss`.
pub fn register(reg: &mut Registry) {
    reg.area(
        "feedback",
        "Bug reports and suggestions to the editor's author, with an anonymous diagnostic package",
    );
    reg.cmd(
        Spec::read(
            "feedback.status",
            "Whether this build can send feedback, the crash a start offers, the last report sent and the crashes the log folder keeps.",
        ),
        |ed, _: NoParams, _| async move { Ok(status(&ed).await) },
    );
    reg.cmd(
        Spec::write(
            "feedback.send",
            "Sends a bug report or a suggestion to the editor's author - only when the user asks for it - with the anonymous diagnostic package and a crash; dry_run answers what would be sent.",
        )
        .example("kind=bug title=\"Gizmo jumps\" text=\"Dragging the move gizmo of a light jumps it 10 m.\" dry_run=true"),
        send_cmd,
    );
    reg.cmd(
        Spec::write(
            "feedback.dismiss",
            "Stops offering a crash of the editor: it stays in the log folder, unsent.",
        ),
        |ed, p: DismissParams, _| async move {
            let crash = find_crash(&ed, &p.crash).await?;
            let (dir, name) = (ed.paths.logs.clone(), crash.name.clone());
            ed.exec.io(move || crashes::mark_handled(&dir, &name)).await.map_err(|e| {
                CmdError::new("failed", format!("The crash could not be marked: {}", gz_base::io_reason(&e)))
            })?;
            if let Some(svc) = FeedbackService::of(&ed) {
                svc.settle_offer(&ed, &crash.name);
            }
            Ok(status(&ed).await)
        },
    );
}
