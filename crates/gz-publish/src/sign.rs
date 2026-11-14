//! The device sign-in: the caller gets the code, it is confirmed on GitHub, and the token goes
//! from GitHub straight into the Credential Manager. It exists only in a build that has the
//! editor's OAuth client id ([`gz_github::Endpoints::device_sign_in`]).

use std::sync::Arc;
use std::time::{Duration, Instant};

use gz_editor::{Cancel, Editor, JobError, JobId};
use gz_github::{DevicePoll, Github};
use serde_json::json;

use crate::job::Publisher;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SignState {
    #[default]
    Idle,
    Starting,
    Waiting,
    Done,
    Failed,
}

impl SignState {
    fn pending(self) -> bool {
        matches!(self, Self::Starting | Self::Waiting)
    }
}

#[derive(Clone, Debug, Default)]
pub struct SignView {
    pub state: SignState,
    pub user_code: String,
    pub uri: String,
    pub error: String,
}

impl Publisher {
    fn sign_set(&self, state: SignState, error: &str) {
        let mut s = self.sign.lock();
        s.state = state;
        s.error = error.to_owned();
    }

    /// Stops a sign-in in flight.
    pub fn sign_stop(&self) {
        if let Some(cancel) = self.sign_cancel.lock().take() {
            cancel.cancel();
        }
        self.sign_set(SignState::Idle, "");
    }

    /// Starts the device sign-in unless one is under way: a job of the editor (the account is
    /// not a project's) that waits for the code to be confirmed on GitHub. Answers the job.
    pub fn sign_in(self: &Arc<Self>, editor: &Arc<Editor>) -> Option<JobId> {
        if self.sign.lock().state.pending() {
            return *self.sign_job.lock();
        }
        self.sign_stop();
        *self.sign.lock() = SignView { state: SignState::Starting, ..SignView::default() };
        let (me, ed) = (self.clone(), editor.clone());
        let started = editor.jobs.start_for_editor(
            "sign_in",
            Some("sign_in"),
            "Signing In to GitHub",
            move |ctx| async move {
                *me.sign_cancel.lock() = Some(ctx.cancel.clone());
                ctx.stage("Waiting for GitHub");
                me.sign_worker(&ed, ctx.cancel.clone(), &ctx).await;
                let s = me.sign.lock().clone();
                match s.state {
                    SignState::Done => Ok(json!({ "signed_in": true })),
                    _ if ctx.cancel.is_cancelled() => Err(JobError::from(gz_task::Cancelled)),
                    _ => Err(JobError::new(if s.error.is_empty() {
                        "The sign-in did not finish.".to_owned()
                    } else {
                        s.error
                    })),
                }
            },
        );
        let id = started.ok();
        *self.sign_job.lock() = id;
        id
    }

    /// The device code while the sign-in waits for it to be confirmed.
    #[must_use]
    pub fn sign_view(&self) -> SignView {
        self.sign.lock().clone()
    }

    async fn sign_worker(&self, editor: &Editor, cancel: Cancel, ctx: &gz_editor::JobCtx) {
        let gh = Github::of(editor);
        let mut code = match gh.device_begin(&cancel).await {
            Ok(code) => code,
            Err(err) => {
                if !cancel.is_cancelled() {
                    self.sign_set(SignState::Failed, &err);
                }
                return;
            }
        };
        {
            let mut s = self.sign.lock();
            s.state = SignState::Waiting;
            s.user_code = code.user_code.clone();
            s.uri = code.verification_uri.clone();
        }
        ctx.stage(format!(
            "Waiting for the code {} to be confirmed at {}",
            code.user_code, code.verification_uri
        ));
        let started = Instant::now();
        loop {
            // a poll that comes early is answered with slow_down and a longer interval
            let wait = Duration::from_millis(u64::from(code.interval) * 1000 + 500);
            tokio::select! {
                () = cancel.cancelled() => {}
                () = tokio::time::sleep(wait) => {}
            }
            if cancel.is_cancelled() {
                return;
            }
            if started.elapsed() > Duration::from_secs(u64::from(code.expires_in)) {
                self.sign_set(SignState::Failed, "The code expired. Start the sign-in again.");
                return;
            }
            match gh.device_poll(&mut code, &cancel).await {
                Ok(DevicePoll::Pending) => {}
                Ok(DevicePoll::Token(token)) => {
                    match gh.save_token(token).await {
                        Ok(()) => self.sign_set(SignState::Done, ""),
                        Err(err) => self.sign_set(SignState::Failed, &err),
                    }
                    return;
                }
                Err(err) => {
                    if !cancel.is_cancelled() {
                        self.sign_set(SignState::Failed, &err);
                    }
                    return;
                }
            }
        }
    }
}
