//! The device sign-in: the caller gets the code, it is confirmed on GitHub, and the token goes
//! from GitHub straight into the Credential Manager. It exists only in a build that has the
//! editor's OAuth client id ([`gz_github::Endpoints::device_sign_in`]).

use std::sync::Arc;
use std::time::{Duration, Instant};

use gz_editor::{Cancel, Editor};
use gz_github::{DevicePoll, Github};
use serde_json::{Value, json};

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
    fn text(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Starting => "starting",
            Self::Waiting => "waiting",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

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

    /// Starts the device sign-in unless one is under way.
    pub fn sign_in(self: &Arc<Self>, editor: &Arc<Editor>) {
        if self.sign.lock().state.pending() {
            return;
        }
        self.sign_stop();
        let cancel = Cancel::new();
        *self.sign_cancel.lock() = Some(cancel.clone());
        *self.sign.lock() = SignView { state: SignState::Starting, ..SignView::default() };
        let (me, ed) = (self.clone(), editor.clone());
        editor.exec.spawn(async move { me.sign_worker(&ed, cancel).await });
    }

    async fn sign_worker(&self, editor: &Editor, cancel: Cancel) {
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

    /// `sign_in_status`: the code while it waits. A finished sign-in is reported once.
    pub fn sign_json(&self) -> (bool, Value) {
        let s = self.sign.lock().clone();
        let waiting = s.state == SignState::Waiting;
        let answer = json!({
            "state": s.state.text(),
            "pending": s.state.pending(),
            "user_code": if waiting { s.user_code.as_str() } else { "" },
            "verification_uri": if waiting { s.uri.as_str() } else { "" },
            "error": s.error,
        });
        if matches!(s.state, SignState::Done | SignState::Failed) {
            *self.sign_cancel.lock() = None;
            self.sign_set(SignState::Idle, "");
        }
        (s.state != SignState::Failed, answer)
    }
}
