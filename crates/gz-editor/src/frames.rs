use std::time::Duration;

use gz_base::SmolStr;
use gz_store::CmdError;
use gz_task::Wake;
use serde::Serialize;
use tokio::sync::watch;

/// What the window says after it presented a frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct FrameStamp {
    /// Counts presented frames; 0 before the first.
    pub frame: u64,
    /// The document id of the tab that frame drew.
    pub active_tab: SmolStr,
    /// Pixels per point of that frame.
    pub scale: f32,
    /// The picture's size in pixels.
    pub width: u32,
    pub height: u32,
}

/// Lets a request wait for frames.
///
/// The editor draws on demand, and a background one draws nothing by itself: an answer given at
/// once would describe the frame from BEFORE the caller's last commands. So a request whose
/// answer is a picture - a screenshot, the UI tree, a scroll position - asks for frames here and
/// is held until they were presented.
pub struct FrameClock {
    tx: watch::Sender<FrameStamp>,
    wake: Wake,
}

impl FrameClock {
    #[must_use]
    pub fn new(wake: Wake) -> Self {
        let (tx, _) = watch::channel(FrameStamp::default());
        Self { tx, wake }
    }

    /// Called by the window after a frame was presented.
    pub fn publish(&self, stamp: FrameStamp) {
        self.tx.send_replace(stamp);
    }

    #[must_use]
    pub fn current(&self) -> FrameStamp {
        self.tx.borrow().clone()
    }

    /// Whether a window draws frames at all. Headless there is none.
    #[must_use]
    pub fn has_window(&self) -> bool {
        self.tx.borrow().frame > 0
    }

    /// Waits until `frames` more frames were presented, asking the window for each.
    pub async fn after(&self, frames: u64) -> Result<FrameStamp, CmdError> {
        let mut rx = self.tx.subscribe();
        let target = rx.borrow().frame + frames.max(1);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        loop {
            let now = rx.borrow_and_update().clone();
            if now.frame >= target {
                return Ok(now);
            }
            self.wake.ping();
            match tokio::time::timeout_at(deadline, rx.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => return Err(CmdError::internal("the window is gone")),
                Err(_) => {
                    return Err(CmdError::new(
                        "no_frame",
                        if now.frame == 0 { "the editor runs without a window" } else { "the editor did not draw a frame in time" },
                    ));
                }
            }
        }
    }
}
