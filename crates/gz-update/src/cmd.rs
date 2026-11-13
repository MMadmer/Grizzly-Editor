//! `update.*`: the editor's own updates (docs/UPDATES.md, section 6).

use std::sync::Arc;

use gz_editor::{CmdError, Editor, JobId, JobState, NoParams, Origin, Registry, Reply, Spec, Value, json};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::service::{State, UpdateService};
use crate::version::Version;

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Wait {
    /// Answer when the job has ended (the default); false answers `{job}` at once.
    #[serde(default = "yes")]
    wait: bool,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Apply {
    /// Start the new version once it is installed. False: the editor ends and the next start is
    /// the new version.
    #[serde(default)]
    restart: bool,
    /// Install the update when the editor exits instead of now (Update on Exit).
    #[serde(default)]
    on_exit: bool,
    /// Save the unsaved work first.
    #[serde(default)]
    save: bool,
    /// Lose the unsaved work.
    #[serde(default)]
    discard: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Set {
    /// Look for a newer version once per start, after the first frame (on by default).
    check_at_startup: bool,
}

fn service(editor: &Editor) -> Result<Arc<UpdateService>, CmdError> {
    UpdateService::of(editor)
        .ok_or_else(|| CmdError::new("refused", "The updater is not running in this editor."))
}

fn refuse_off(svc: &UpdateService) -> Result<(), CmdError> {
    if svc.install.is_some() { Ok(()) } else { Err(CmdError::new("refused", svc.text())) }
}

/// What `update.status` answers.
#[must_use]
pub fn status(editor: &Editor) -> Value {
    match UpdateService::of(editor) {
        Some(svc) => svc.status(editor),
        None => json!({
            "version": Version::compiled().to_string(),
            "state": State::Off.word(),
            "text": "This is a development build: it does not update itself.",
        }),
    }
}

/// Runs the update job and waits for it; the job's error is the command's.
async fn run(
    editor: &Arc<Editor>,
    svc: &Arc<UpdateService>,
    then_download: bool,
    wait: bool,
    origin: Origin,
) -> Reply {
    let id: JobId = match svc.start_job(editor, then_download, origin) {
        Ok(id) => id,
        // one update job at a time: the one that runs answers for this call
        Err(e) if e.code == "busy" => match editor.jobs.running_in(crate::service::GROUP) {
            Some(j) if wait => j.id,
            _ => return Err(e),
        },
        Err(e) => return Err(e),
    };
    if !wait {
        return Ok(json!({ "job": id }));
    }
    let info =
        editor.jobs.wait(id).await.ok_or_else(|| CmdError::internal("the update job was forgotten"))?;
    match info.state {
        JobState::Failed | JobState::Cancelled => {
            Err(CmdError::new("failed", info.error.unwrap_or_else(|| "The update stopped.".to_owned()))
                .with_data(svc.status(editor)))
        }
        _ => Ok(svc.status(editor)),
    }
}

async fn apply(editor: Arc<Editor>, p: Apply, origin: Origin) -> Reply {
    let svc = service(&editor)?;
    refuse_off(&svc)?;
    if p.on_exit {
        if svc.state() != State::Ready {
            return Err(CmdError::new(
                "refused",
                "No update is ready to install: download it first (update.download).",
            ));
        }
        svc.set_on_exit(&editor, true);
        return Ok(svc.status(&editor));
    }
    match svc.state() {
        State::Ready => {}
        State::Applying => return Err(CmdError::new("refused", "The update is being installed already.")),
        _ => {
            run(&editor, &svc, true, true, origin).await?;
            match svc.state() {
                State::Ready => {}
                State::Idle => {
                    let mut out = svc.status(&editor);
                    out["updated"] = json!(false);
                    return Ok(out);
                }
                _ => return Err(CmdError::new("refused", svc.text()).with_data(svc.status(&editor))),
            }
        }
    }
    if let Some(job) = UpdateService::blocking_job(&editor) {
        return Err(CmdError::new("refused", format!("The update waits for {job} to end."))
            .with_data(json!({ "running": job })));
    }
    if p.save {
        editor.save_all().await?;
    } else {
        editor.refuse_unsaved(p.discard)?;
        if p.discard {
            editor.discard_all().await?;
        }
    }
    let from = svc.apply_now(&editor, p.restart).await?;
    UpdateService::quit_soon(&editor);
    let mut out = svc.status(&editor);
    out["updated"] = json!(true);
    out["from"] = json!(from);
    out["restart"] = json!(p.restart);
    Ok(out)
}

/// Registers `update.status`, `update.check`, `update.download`, `update.stop`, `update.apply`
/// and `update.set`.
pub fn register(reg: &mut Registry) {
    reg.area("update", "The editor's own updates");
    reg.cmd_sync(
        Spec::read(
            "update.status",
            "What the updater knows: the versions, its state, the download and the notes.",
        ),
        |ed: &Editor, _: &NoParams, _| Ok(status(ed)),
    );
    reg.cmd(
        Spec::write(
            "update.check",
            "Looks for a newer version of the editor and what its update would download.",
        )
        .example("wait=false"),
        |ed, p: Wait, origin| async move {
            let svc = service(&ed)?;
            refuse_off(&svc)?;
            run(&ed, &svc, false, p.wait, origin).await
        },
    );
    reg.cmd(
        Spec::write(
            "update.download",
            "Downloads the update into the installation's stage; a stopped or cut download continues where it was.",
        ),
        |ed, p: Wait, origin| async move {
            let svc = service(&ed)?;
            refuse_off(&svc)?;
            if svc.state() == State::Ready {
                return Ok(svc.status(&ed));
            }
            run(&ed, &svc, true, p.wait, origin).await
        },
    );
    reg.cmd_sync(
        Spec::write("update.stop", "Stops the update's download. What is downloaded stays for the next one."),
        |ed: &Editor, _: &NoParams, _| {
            let svc = service(ed)?;
            let stopping = svc.stop(ed);
            let mut out = svc.status(ed);
            out["stopping"] = json!(stopping);
            Ok(out)
        },
    );
    reg.cmd(
        Spec::write(
            "update.apply",
            "Installs the update - checked and downloaded first when need be - and ends the editor; restart starts the new version.",
        )
        .destroys_with(&["discard"])
        .example("restart=true"),
        apply,
    );
    reg.cmd_sync(
        Spec::write(
            "update.set",
            "Changes the update preferences: whether the editor looks for a newer version at startup.",
        ),
        |ed: &Editor, p: &Set, _| {
            let svc = service(ed)?;
            svc.set_check_at_startup(ed, p.check_at_startup)?;
            Ok(svc.status(ed))
        },
    );
}
