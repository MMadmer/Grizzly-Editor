//! `job.*`: every long piece of work of the editor, whoever started it. A command that starts
//! a job waits for it unless told `wait=false`; these commands are how a caller follows, waits
//! for or stops any of them.

use gz_store::CmdError;
use gz_task::{JobId, JobInfo};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::editor::Editor;
use crate::registry::{Registry, Reply};
use crate::spec::Spec;

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct List {
    /// Only the jobs that still run.
    #[serde(default)]
    running: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct One {
    /// The job's id, as `job.list` and a command started with `wait=false` answer it.
    job: u64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Wait {
    /// The job's id. Left out: every job that runs now.
    #[serde(default)]
    job: Option<u64>,
}

fn no_job(id: u64) -> CmdError {
    CmdError::not_found(format!("there is no job {id}; job.list names the jobs"))
}

/// What a caller gets of a job: its card, and its result or error once it ended.
fn answer(info: &JobInfo) -> Value {
    serde_json::to_value(info).unwrap_or(Value::Null)
}

impl Registry {
    /// Registers `job.list`, `job.get`, `job.wait` and `job.stop`.
    pub fn install_jobs(&mut self) {
        self.area("job", "Long work in the background: list, follow, wait for, stop");
        self.cmd_sync(
            Spec::read("job.list", "The jobs that run and the last ones that ended."),
            |ed: &Editor, p: &List, _| {
                let jobs: Vec<JobInfo> =
                    ed.jobs.snapshot().into_iter().filter(|j| !p.running || j.running()).collect();
                Ok(json!({ "jobs": jobs }))
            },
        );
        self.cmd_sync(
            Spec::read("job.get", "One job: its stage and progress, its result or error once it ended."),
            |ed: &Editor, p: &One, _| {
                ed.jobs.get(JobId(p.job)).map(|j| answer(&j)).ok_or_else(|| no_job(p.job))
            },
        );
        self.cmd(
            Spec::read(
                "job.wait",
                "Waits until a job - or every running job - has ended and answers what it came to.",
            ),
            |ed, p: Wait, _| async move { wait(&ed, p).await },
        );
        self.cmd_sync(
            Spec::write("job.stop", "Asks a job to stop. It ends as cancelled at its next safe point."),
            |ed: &Editor, p: &One, _| {
                let info = ed.jobs.get(JobId(p.job)).ok_or_else(|| no_job(p.job))?;
                let asked = info.running() && ed.jobs.cancel(JobId(p.job));
                Ok(json!({ "job": p.job, "stopping": asked, "state": info.state }))
            },
        );
    }
}

async fn wait(ed: &Editor, p: Wait) -> Reply {
    if let Some(id) = p.job {
        if ed.jobs.get(JobId(id)).is_none() {
            return Err(no_job(id));
        }
        let info = ed.jobs.wait(JobId(id)).await.ok_or_else(|| no_job(id))?;
        return Ok(answer(&info));
    }
    // every job that runs now, and the ones they start meanwhile
    let mut ended = Vec::new();
    loop {
        let running: Vec<JobId> =
            ed.jobs.snapshot().into_iter().filter(JobInfo::running).map(|j| j.id).collect();
        if running.is_empty() {
            break;
        }
        for id in running {
            if let Some(info) = ed.jobs.wait(id).await {
                ended.push(info);
            }
        }
    }
    Ok(json!({ "jobs": ended }))
}
