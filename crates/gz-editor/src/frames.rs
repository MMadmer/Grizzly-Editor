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
    /// What the frame was drawn for (`frame_pacing`'s `ui_frame_causes`): `repaint <file>:<line>`
    /// (egui asked, from that line), `wake` (another thread pinged the window), `input`.
    pub cause: SmolStr,
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
    /// The number of the last frame that began (`begin`): a frame under way when a request comes
    /// may have drawn the state from before the request's commands, so it does not count for it.
    started: std::sync::atomic::AtomicU64,
    wake: Wake,
    times: parking_lot::Mutex<std::collections::VecDeque<FrameTime>>,
    /// Every frame's cause, counted since the start.
    causes: parking_lot::Mutex<std::collections::BTreeMap<SmolStr, u64>>,
}

impl FrameClock {
    #[must_use]
    pub fn new(wake: Wake) -> Self {
        let (tx, _) = watch::channel(FrameStamp::default());
        Self {
            tx,
            started: std::sync::atomic::AtomicU64::new(0),
            wake,
            times: parking_lot::Mutex::new(std::collections::VecDeque::with_capacity(KEPT_TIMES)),
            causes: parking_lot::Mutex::new(std::collections::BTreeMap::new()),
        }
    }

    /// Called by the window when a frame begins, before it reads any state; answers the number
    /// the frame will be published with.
    pub fn begin(&self) -> u64 {
        self.started.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1
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
        *self.causes.lock().entry(stamp.cause.clone()).or_insert(0) += 1;
        self.tx.send_replace(stamp);
    }

    /// The times of the frames presented after `frame` that the clock still keeps (the last
    /// 4096), oldest first.
    #[must_use]
    pub fn times_after(&self, frame: u64) -> Vec<FrameTime> {
        self.times.lock().iter().filter(|t| t.frame > frame).copied().collect()
    }

    /// How many frames each cause drew since the start ([`FrameStamp::cause`]).
    #[must_use]
    pub fn causes(&self) -> std::collections::BTreeMap<SmolStr, u64> {
        self.causes.lock().clone()
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

    /// Waits until `frames` frames that BEGAN after this call were presented, asking the window
    /// for each: a frame already under way does not count, whatever it shows.
    pub async fn after(&self, frames: u64) -> Result<FrameStamp, CmdError> {
        let mut rx = self.tx.subscribe();
        let begun = self.started.load(std::sync::atomic::Ordering::Acquire).max(rx.borrow().frame);
        let target = begun + frames.max(1);
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

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn stamp(frame: u64) -> FrameStamp {
        FrameStamp { frame, ..FrameStamp::default() }
    }

    #[tokio::test]
    async fn a_frame_under_way_does_not_answer_a_request() {
        let clock = std::sync::Arc::new(FrameClock::new(Wake::default()));
        // frame 1 has begun - it may read the state from before the request's commands
        assert_eq!(clock.begin(), 1);
        let c2 = clock.clone();
        let held = tokio::spawn(async move { c2.after(1).await.unwrap().frame });
        tokio::time::sleep(Duration::from_millis(30)).await;
        clock.publish(stamp(1));
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!held.is_finished(), "frame 1 began before the request");
        assert_eq!(clock.begin(), 2);
        clock.publish(stamp(2));
        assert_eq!(held.await.unwrap(), 2);
        // nothing under way: the next frame answers
        let c3 = clock.clone();
        let next = tokio::spawn(async move { c3.after(1).await.unwrap().frame });
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(clock.begin(), 3);
        clock.publish(stamp(3));
        assert_eq!(next.await.unwrap(), 3);
    }
}
