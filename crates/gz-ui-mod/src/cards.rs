//! The Task Center's card of a build and of a packaging (docs/UI_DESIGN.md, section 5, "Tasks"):
//! the steps (`Check` - `Copy` - `Install`, or `Pack`), the stage, how far in bytes and files,
//! the file at hand, and Stop under the job's own name.
//!
//! `gz-build` and `gz-release` keep each job's row as data ([`gz_build::Builds::status`],
//! [`gz_release::Releases::status`]); this pump publishes it into `gz_app::TaskCardTable` for
//! every running `build` and `package` job - the ones MCP started too (`progress` lists their
//! steps), marked as such so their card never opens by itself. A card is written only when it
//! changed.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gz_app::TaskCard;
use gz_build::{BuildStatus, Builds};
use gz_editor::{Editor, JobId, JobInfo, Value};
use gz_release::Releases;
use parking_lot::Mutex;

/// How often the running jobs' rows are looked at: often enough that `progress` over MCP sees a
/// copy of a few hundred milliseconds move.
const PERIOD: Duration = Duration::from_millis(25);

/// The pump: one task for the editor's life, started on first use.
#[derive(Default)]
pub struct CardPump {
    started: AtomicBool,
    /// Jobs the window started: their card opens.
    from_ui: Mutex<BTreeSet<JobId>>,
    published: Mutex<BTreeMap<JobId, Value>>,
}

/// Starts the pump once, and hands the build command the window's Build Into Game: a person
/// who clicks it is asked what the build needs and sees its result.
pub fn ensure(editor: &Arc<Editor>) {
    let pump = editor.services.get_or_init(CardPump::default);
    if pump.started.swap(true, Ordering::SeqCst) {
        return;
    }
    gz_build::WindowBuilds::of(editor).provide(|ed| {
        if gz_app::UiQueue::of(ed).is_err() {
            return false;
        }
        let ed = ed.clone();
        let _task = ed.exec.clone().spawn(async move {
            let _ = gz_app::on_ui(&ed, |shell, ctx| {
                shell.with_cx(ctx, crate::build::request_build_into_game);
                Ok(())
            })
            .await;
        });
        true
    });
    let weak = Arc::downgrade(editor);
    let _task = editor.exec.spawn(async move {
        loop {
            tokio::time::sleep(PERIOD).await;
            let Some(ed) = weak.upgrade() else { break };
            pump.tick(&ed);
        }
    });
}

/// A job the window started: its card is there before the shell first looks, so it opens.
pub fn started_from_ui(editor: &Arc<Editor>, job: JobId) {
    ensure(editor);
    let pump = editor.services.get_or_init(CardPump::default);
    pump.from_ui.lock().insert(job);
    if let Some(info) = editor.jobs.get(job) {
        pump.publish(editor, &info);
    }
}

/// The card of a job's row; the row's defaults before the job has said anything.
#[must_use]
pub fn card_of(kind: &str, status: Option<BuildStatus>, from_mcp: bool) -> TaskCard {
    let (steps, stop_label, stop_tip) = match kind {
        "package" => (
            vec!["Check", "Copy", "Pack"],
            "Stop Packaging",
            "Stops the packaging. Nothing is written to the release folder before the packages are done.",
        ),
        _ => (
            vec!["Check", "Copy", "Install"],
            "Stop Build",
            "Stops the build. The module in the target stays as it was.",
        ),
    };
    let mut card = TaskCard {
        key: kind.to_owned(),
        steps: steps.into_iter().map(str::to_owned).collect(),
        from_mcp,
        stop_label: stop_label.to_owned(),
        stop_tip: stop_tip.to_owned(),
        ..TaskCard::default()
    };
    if let Some(s) = status {
        card.subtitle.clone_from(&s.subtitle);
        card.numbers = s.numbers();
        card.file.clone_from(&s.file);
        card.step = i32::try_from(s.step).unwrap_or(0);
        if !s.steps.is_empty() {
            card.steps = s.steps.iter().map(|x| (*x).to_owned()).collect();
        }
        if !s.stop_label.is_empty() {
            s.stop_label.clone_into(&mut card.stop_label);
            s.stop_tip.clone_into(&mut card.stop_tip);
        }
    }
    card
}

impl CardPump {
    fn status(editor: &Editor, kind: &str) -> Option<BuildStatus> {
        match kind {
            "build" => Builds::of(editor).status(),
            "package" => Releases::of(editor).status(editor),
            _ => None,
        }
    }

    fn publish(&self, editor: &Editor, info: &JobInfo) {
        let from_mcp = !self.from_ui.lock().contains(&info.id);
        let card = card_of(&info.kind, Self::status(editor, &info.kind), from_mcp);
        let value = serde_json::to_value(&card).unwrap_or(Value::Null);
        let mut published = self.published.lock();
        if published.get(&info.id) != Some(&value) {
            published.insert(info.id, value);
            drop(published);
            card.publish(editor, info.id);
        }
    }

    fn tick(&self, editor: &Editor) {
        let running: Vec<JobInfo> = editor
            .jobs
            .snapshot()
            .into_iter()
            .filter(|j| j.running() && matches!(j.kind.as_str(), "build" | "package"))
            .collect();
        for job in &running {
            self.publish(editor, job);
        }
        let alive = |id: &JobId| running.iter().any(|j| j.id == *id);
        self.published.lock().retain(|id, _| alive(id));
        self.from_ui.lock().retain(alive);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_build_card_names_its_steps_and_its_stop() {
        let card = card_of("build", None, true);
        assert_eq!(card.steps, ["Check", "Copy", "Install"]);
        assert_eq!(card.stop_label, "Stop Build");
        assert!(card.from_mcp);
        let status = BuildStatus {
            steps: vec!["Check", "Copy"],
            step: 1,
            total: 2 << 20,
            done: 1 << 20,
            files: 1,
            files_total: 2,
            stop_label: "Stop Export",
            ..BuildStatus::default()
        };
        let card = card_of("build", Some(status), false);
        assert_eq!(card.steps, ["Check", "Copy"]);
        assert_eq!(card.step, 1);
        assert_eq!(card.stop_label, "Stop Export");
        assert!(card.numbers.contains(" of ") && card.numbers.contains("files"));
    }

    #[test]
    fn a_packaging_card_packs() {
        assert_eq!(card_of("package", None, false).steps, ["Check", "Copy", "Pack"]);
    }
}
