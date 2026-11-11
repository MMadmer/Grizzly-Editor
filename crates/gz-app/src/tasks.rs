//! The Task Center: what jobs say of themselves beyond their stage and fraction, and the
//! `progress` answer.
//!
//! A job attaches its card as JSON in [`TaskCardTable`], an `Editor` service keyed by a std
//! type on purpose: an area crate below the UI publishes without depending on this crate -
//! `editor.services.get_or_init::<TaskCardTable>(Default::default).lock().insert(id, json!({..}))`.
//! The fields are those of [`TaskCard`]; every one is optional.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use gz_editor::{Editor, JobId, JobInfo};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// Where jobs publish their cards.
pub type TaskCardTable = Mutex<BTreeMap<JobId, Value>>;

/// What a job's card shows beyond its title, stage and fraction.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskCard {
    /// Rows of one key are one row: a packaging and the upload that follows it.
    pub key: String,
    /// What it is about: the module, the target folder.
    pub subtitle: String,
    /// `Check` - `Copy` - `Install`.
    pub steps: Vec<String>,
    /// The current step, 0-based.
    pub step: i32,
    /// "2.1 of 4.6 GB · 3412 of 7146 files".
    pub numbers: String,
    /// The file at hand.
    pub file: String,
    /// What is left, when the job can tell.
    pub eta_ms: u64,
    /// Started over MCP: its card never opens by itself.
    pub from_mcp: bool,
    /// The Stop button's label: "Stop Publishing". Empty: "Stop".
    pub stop_label: String,
    pub stop_tip: String,
}

impl TaskCard {
    /// Publishes this card for a job.
    pub fn publish(&self, editor: &Editor, job: JobId) {
        let value = serde_json::to_value(self).unwrap_or(Value::Null);
        editor.services.get_or_init::<TaskCardTable>(Default::default).lock().insert(job, value);
        editor.exec.wake().ping();
    }
}

/// The card a job published, or an empty one.
#[must_use]
pub fn card_of(editor: &Editor, job: JobId) -> Option<TaskCard> {
    let table = editor.services.get::<TaskCardTable>()?;
    let value = table.lock().get(&job).cloned()?;
    serde_json::from_value(value).ok()
}

/// Forgets the cards of jobs that ended.
pub(crate) fn sweep(editor: &Editor, running: &[JobInfo]) {
    if let Some(table) = editor.services.get::<TaskCardTable>() {
        table.lock().retain(|id, _| running.iter().any(|j| j.id == *id));
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// "01:08", "1:02:03".
#[must_use]
pub fn format_elapsed(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

/// One running job, as `progress` lists it under `background`.
pub(crate) fn task_json(job: &JobInfo, card: Option<&TaskCard>, cancel_requested: bool, now: u64) -> Value {
    let mut out = Map::new();
    out.insert("text".into(), json!(job.title));
    out.insert("detail".into(), json!(job.stage));
    let empty = TaskCard::default();
    let c = card.unwrap_or(&empty);
    let key = if c.key.is_empty() { format!("{}:{}", job.kind, job.id.0) } else { c.key.clone() };
    out.insert("key".into(), json!(key));
    out.insert("subtitle".into(), json!(c.subtitle));
    out.insert("numbers".into(), json!(c.numbers));
    out.insert("file".into(), json!(c.file));
    out.insert("steps".into(), json!(c.steps.join(", ")));
    out.insert("step".into(), json!(c.step));
    out.insert("eta_ms".into(), json!(c.eta_ms));
    out.insert("note".into(), json!(if c.from_mcp { "started over MCP" } else { "started from the UI" }));
    let fraction = job.fraction.unwrap_or(0.0);
    out.insert("current".into(), json!(round3(fraction * 100.0)));
    out.insert("total".into(), json!(if job.fraction.is_some() { 100.0 } else { 0.0 }));
    out.insert("fraction".into(), json!((f64::from(fraction) * 1e6).round() / 1e6));
    out.insert("elapsed_ms".into(), json!(now.saturating_sub(job.started_ms)));
    out.insert("depth".into(), json!(0));
    out.insert("determinate".into(), json!(job.fraction.is_some()));
    out.insert("cancelable".into(), json!(true));
    out.insert("cancel_requested".into(), json!(cancel_requested));
    Value::Object(out)
}

fn round3(v: f32) -> f64 {
    (f64::from(v) * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::{TaskCard, format_elapsed};

    #[test]
    fn elapsed_reads_like_a_clock() {
        assert_eq!(format_elapsed(0), "00:00");
        assert_eq!(format_elapsed(68_000), "01:08");
        assert_eq!(format_elapsed(3_723_000), "1:02:03");
    }

    #[test]
    fn a_card_reads_from_partial_json() {
        let card: TaskCard =
            serde_json::from_value(serde_json::json!({ "steps": ["Check", "Copy"], "step": 1 }))
                .unwrap_or_default();
        assert_eq!(card.steps.len(), 2);
        assert_eq!(card.step, 1);
        assert!(!card.from_mcp);
    }
}
