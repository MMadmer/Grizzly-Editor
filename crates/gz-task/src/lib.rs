//! Where work runs.
//!
//! - the **async runtime** orchestrates: store actors, the MCP server, the network, timers;
//! - the **CPU pool** computes: pure functions over snapshots, data-parallel;
//! - the **IO pool** touches files: sized to overlap latency, because a start costs the number of
//!   file operations and not the bytes;
//! - a **job** is anything long the author can watch and stop.
//!
//! Nothing here is global: an [`Executors`] handle is passed down from the application.

mod cancel;
mod exec;
mod gate;
mod jobs;
mod wake;

pub use cancel::{Cancel, Cancelled};
pub use exec::{ExecOptions, Executors, Runtime};
pub use gate::Gate;
pub use jobs::{
    JobBusy, JobCtx, JobError, JobId, JobInfo, JobScope, JobState, Jobs, Owner, current_scope, in_scope,
    in_scope_sync,
};
pub use wake::Wake;

/// The text of a panic payload, for a job or a command that failed by panicking.
#[must_use]
pub fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_owned()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "panic".to_owned()
    }
}

#[cfg(test)]
mod tests;
