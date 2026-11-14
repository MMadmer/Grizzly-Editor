//! The stall monitor: the proof that nothing blocks the UI thread (docs/plan section 6,
//! docs/UI_DESIGN.md section 5).
//!
//! The window times every UI pass and every event it handles. One that takes longer than
//! [`STALL_MS`] is a **stall**: kept here (the last [`KEEP`]) with its milliseconds, when it
//! happened, the tab that was up and what ran - the phase (`ui pass`, `event: resized`, `command:
//! <its function>` when a command's work on the UI thread ran in it) - and written to the log as
//! `[stall] 412 ms in <tab>: <what ran>`. With `GRIZZLY_DEBUG_STALLS=1` that line is a warning
//! the status bar counts; otherwise a detail. The first frame of a start is no stall: it makes
//! the fonts' atlas and the first pipelines. `perf.stalls` answers the list.
//!
//! **A stall is the UI thread waiting, not the machine being busy.** A thread of its own - the
//! heartbeat - sleeps ten milliseconds at a time and notes how late each wake-up was. When the
//! heartbeat itself could not run for most of a long pass, no thread of the editor had a
//! processor (a build on every core, twenty test editors side by side): that pass is kept apart
//! as `starved` and is no stall. A UI thread that waits on a file or a lock leaves the heartbeat
//! on time, and is one.

use std::collections::VecDeque;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use gz_editor::{Editor, SmolStr};
use parking_lot::Mutex;
use serde::Serialize;

/// Longer than this on the UI thread is a stall.
pub const STALL_MS: f32 = 100.0;
/// How many stalls are kept.
pub const KEEP: usize = 256;

/// One stall.
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Stall {
    pub ms: f32,
    /// When it ended, in milliseconds since 1970.
    pub when: u64,
    /// The tab that was up.
    pub tab: SmolStr,
    /// What ran: `ui pass`, `event: <kind>`, `command: <function>`.
    pub during: String,
}

/// How often the heartbeat wakes.
const BEAT: Duration = Duration::from_millis(10);
/// How long the heartbeat's late wake-ups are remembered.
const BEATS_KEPT: Duration = Duration::from_secs(5);

/// The wake-ups of the heartbeat that came late: when each came and how long after the one before.
#[derive(Default)]
struct Beats {
    late: Mutex<VecDeque<(Instant, f32)>>,
}

impl Beats {
    /// Runs until the stalls it beats for are gone.
    fn run(owner: Weak<Stalls>) {
        let mut last = Instant::now();
        loop {
            std::thread::sleep(BEAT);
            let Some(stalls) = owner.upgrade() else { return };
            let now = Instant::now();
            let gap = now.duration_since(last).as_secs_f32() * 1000.0;
            last = now;
            // a wake-up within a timer tick or two of its time is on time
            if gap > 40.0 {
                let mut late = stalls.beats.late.lock();
                late.push_back((now, gap));
                while late.front().is_some_and(|(at, _)| now.duration_since(*at) > BEATS_KEPT) {
                    late.pop_front();
                }
            }
        }
    }

    /// The longest the heartbeat could not run during the `ms` that ended at `end`.
    fn longest_gap(&self, end: Instant, ms: f32) -> f32 {
        let span = Duration::from_secs_f32(ms / 1000.0);
        self.late
            .lock()
            .iter()
            // a gap that ended inside the span, or just after it (the wake-up that ends a gap comes
            // when the processor is back, which may be a moment after the pass)
            .filter(|(at, _)| {
                *at + Duration::from_millis(50) >= end.checked_sub(span).unwrap_or(end)
                    && *at <= end + Duration::from_millis(50)
            })
            .map(|(_, gap)| *gap)
            .fold(0.0, f32::max)
    }
}

/// The stalls of this run, an `Editor` service.
#[derive(Default)]
pub struct Stalls {
    list: Mutex<VecDeque<Stall>>,
    /// Long passes of a machine with no processor to give: kept to be seen, never counted.
    starved: Mutex<VecDeque<Stall>>,
    worst: Mutex<f32>,
    beats: Beats,
}

impl Stalls {
    pub fn of(editor: &Editor) -> Arc<Stalls> {
        let mut made = false;
        let stalls = editor.services.get_or_init(|| {
            made = true;
            Stalls::default()
        });
        if made {
            let owner = Arc::downgrade(&stalls);
            // the heartbeat that tells a waiting UI thread from a starved process
            let _ = std::thread::Builder::new()
                .name("stall-heartbeat".to_owned())
                .spawn(move || Beats::run(owner));
        }
        stalls
    }

    /// Records `ms` of `during` on `tab` when it is a stall; answers whether it was one.
    pub fn note(&self, editor: &Editor, ms: f32, tab: &str, during: &str) -> bool {
        if ms <= STALL_MS {
            return false;
        }
        let when = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let stall =
            Stall { ms: (ms * 10.0).round() / 10.0, when, tab: SmolStr::new(tab), during: during.to_owned() };
        // nobody in this process ran for most of it: the machine was busy, the UI thread did not wait
        let gap = self.beats.longest_gap(Instant::now(), ms);
        if gap >= ms * 0.5 {
            let mut l = self.starved.lock();
            if l.len() == KEEP {
                l.pop_front();
            }
            l.push_back(stall);
            editor.log.detail(format!(
                "[starved] {ms:.0} ms in {}: {during} - no thread of the editor ran for {gap:.0} ms of it",
                if tab.is_empty() { "the Start page" } else { tab }
            ));
            return false;
        }
        {
            let mut l = self.list.lock();
            if l.len() == KEEP {
                l.pop_front();
            }
            l.push_back(stall);
        }
        {
            let mut w = self.worst.lock();
            *w = w.max(ms);
        }
        let line = format!(
            "[stall] {ms:.0} ms in {}: {during}",
            if tab.is_empty() { "the Start page" } else { tab }
        );
        if debug_on() {
            editor.log.warn(line);
        } else {
            editor.log.detail(line);
        }
        true
    }

    /// The stalls kept, oldest first.
    #[must_use]
    pub fn list(&self) -> Vec<Stall> {
        self.list.lock().iter().cloned().collect()
    }

    /// The long passes that were the machine's doing, oldest first.
    #[must_use]
    pub fn starved(&self) -> Vec<Stall> {
        self.starved.lock().iter().cloned().collect()
    }

    /// The longest one since the start (or the last clear).
    #[must_use]
    pub fn worst_ms(&self) -> f32 {
        *self.worst.lock()
    }

    pub fn clear(&self) {
        self.list.lock().clear();
        self.starved.lock().clear();
        *self.worst.lock() = 0.0;
    }
}

/// `GRIZZLY_DEBUG_STALLS=1`: a stall is a warning the status bar counts.
fn debug_on() -> bool {
    std::env::var("GRIZZLY_DEBUG_STALLS").is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

/// The phases of one frame and how long each took: the longest names what a stall ran.
#[derive(Default)]
pub(crate) struct FramePhases {
    phases: Vec<(String, f32)>,
}

impl FramePhases {
    pub(crate) fn add(&mut self, what: impl Into<String>, ms: f32) {
        self.phases.push((what.into(), ms));
    }

    /// The longest phase, or `ui pass`.
    pub(crate) fn longest(&self) -> String {
        self.phases
            .iter()
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map_or_else(|| "ui pass".to_owned(), |(w, ms)| format!("{w} ({ms:.0} ms)"))
    }

    pub(crate) fn clear(&mut self) {
        self.phases.clear();
    }
}

/// What a queued closure of a command is called in a stall: the command's function, read off
/// the closure's type (`gz_app::cmds::ui_tab::{{closure}}` is `command: gz_app::cmds::ui_tab`).
#[must_use]
pub fn label_of<F>() -> &'static str {
    std::any::type_name::<F>()
}

/// The readable form of a closure's type name.
#[must_use]
pub fn command_of(type_name: &str) -> String {
    let head = type_name.split("::{{closure}}").next().unwrap_or(type_name);
    format!("command: {head}")
}

#[cfg(test)]
mod tests {
    use super::{Stalls, command_of};

    #[test]
    fn a_long_pass_is_a_stall_and_a_short_one_is_not() {
        let rt =
            gz_task::Runtime::start(gz_task::ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 1 })
                .expect("runtime");
        let ed = gz_editor::Editor::new(
            gz_editor::EditorOptions::default(),
            rt.exec(),
            gz_log::Log::memory(),
            gz_editor::Registry::new(),
        );
        let s = Stalls::of(&ed);
        assert!(!s.note(&ed, 40.0, "level", "ui pass"));
        assert!(s.note(&ed, 412.0, "quest:q.nqasset", "command: gz_app::cmds::ui_tab"));
        let l = s.list();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].tab, "quest:q.nqasset");
        assert!(ed.log.tail(1)[0].text.starts_with("[stall] 412 ms in quest:q.nqasset"));
        assert!((s.worst_ms() - 412.0).abs() < 0.01);
        s.clear();
        assert!(s.list().is_empty());
        // a pass during which the heartbeat itself could not run is the machine's, not a stall
        s.beats.late.lock().push_back((std::time::Instant::now(), 180.0));
        assert!(!s.note(&ed, 200.0, "level", "ui pass"));
        assert!(s.list().is_empty());
        assert_eq!(s.starved().len(), 1);
        // a heartbeat that was late only briefly does not excuse a long wait
        s.clear();
        s.beats.late.lock().clear();
        s.beats.late.lock().push_back((std::time::Instant::now(), 60.0));
        assert!(s.note(&ed, 400.0, "level", "ui pass"));
        s.clear();
        assert_eq!(
            command_of("gz_app::cmds::ui_tab::{{closure}}::{{closure}}"),
            "command: gz_app::cmds::ui_tab"
        );
    }
}
