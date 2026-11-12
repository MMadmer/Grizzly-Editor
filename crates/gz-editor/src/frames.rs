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
    /// The frame's own CPU time: input, the UI pass, tessellation, recording - everything but
    /// the waits below.
    pub work_ms: f32,
    /// What the frame waited for the presentation: the swap chain's image (vsync, the frame
    /// latency), the submission, the present.
    pub wait_ms: f32,
}

/// One presented frame's times, kept for `frame_pacing`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameTime {
    pub frame: u64,
    pub work_ms: f32,
    pub wait_ms: f32,
}

/// How many frames the clock keeps the times of.
const KEPT_TIMES: usize = 4096;

/// Lets a request wait for frames.
///
/// The editor draws on demand, and a background one draws nothing by itself: an answer given at
/// once would describe the frame from BEFORE the caller's last commands. So a request whose
/// answer is a picture - a screenshot, the UI tree, a scroll position - asks for frames here and
/// is held until they were presented.
pub struct FrameClock {
    tx: watch::Sender<FrameStamp>,
    wake: Wake,
    times: parking_lot::Mutex<std::collections::VecDeque<FrameTime>>,
}

impl FrameClock {
    #[must_use]
    pub fn new(wake: Wake) -> Self {
        let (tx, _) = watch::channel(FrameStamp::default());
        Self {
            tx,
            wake,
            times: parking_lot::Mutex::new(std::collections::VecDeque::with_capacity(KEPT_TIMES)),
        }
    }

    /// Called by the window after a frame was presented.
    pub fn publish(&self, stamp: FrameStamp) {
        {
            let mut t = self.times.lock();
            if t.len() == KEPT_TIMES {
                t.pop_front();
            }
            t.push_back(FrameTime { frame: stamp.frame, work_ms: stamp.work_ms, wait_ms: stamp.wait_ms });
        }
        self.tx.send_replace(stamp);
    }

    /// The times of the frames presented after `frame` that the clock still keeps (the last
    /// 4096), oldest first.
    #[must_use]
    pub fn times_after(&self, frame: u64) -> Vec<FrameTime> {
        self.times.lock().iter().filter(|t| t.frame > frame).copied().collect()
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
                        if now.frame == 0 {
                            "the editor runs without a window"
                        } else {
                            "the editor did not draw a frame in time"
                        },
                    ));
                }
            }
        }
    }
}
