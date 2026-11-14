//! The Problems panel (`Problems###level.problems`, beside the Output Log): what the level's last
//! builds found, one sentence per row with its object and its step. A click selects the object
//! and frames it; a problem with only a place frames the place.
//!
//! The list is `level.problems`, read off the UI thread when the panel comes up, when a build of
//! the level ends and when Clear Problems answered - never per frame and never by polling.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use egui::Ui;
use gz_editor::{Args, Editor, Value, json};
use gz_ui::rows::{Col, Table};
use gz_ui::text::Clip;
use gz_ui::{Font, Icon, TipExt, Tone, Tx, palette, state};
use parking_lot::Mutex;

/// The panel's MCP id.
pub const PANEL: &str = "problems";

/// One row of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub text: String,
    pub error: bool,
    pub object: Option<String>,
    pub position: Option<[f32; 3]>,
    /// The step's title: "Geometry".
    pub step: String,
}

impl Problem {
    /// A row of `level.problems`.
    #[must_use]
    pub fn of(v: &Value) -> Self {
        let step = v.get("step").and_then(Value::as_str).unwrap_or_default();
        let title = gz_levelbuild::Step::ORDER
            .into_iter()
            .find(|s| s.name() == step)
            .map_or_else(|| step.to_owned(), |s| s.title().to_owned());
        let position = v.get("position").and_then(Value::as_array).and_then(|a| {
            let n: Vec<f32> = a.iter().filter_map(Value::as_f64).map(|x| x as f32).collect();
            (n.len() == 3).then(|| [n[0], n[1], n[2]])
        });
        Self {
            // a line of the compiler keeps the marks of its log; the panel shows the sentence
            text: v
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim_start_matches(['*', '!', ' '])
                .to_owned(),
            error: v.get("severity").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case("error")),
            object: v.get("object").and_then(Value::as_str).map(str::to_owned),
            position,
            step: title,
        }
    }
}

/// The list as last read, and what it was read for.
#[derive(Default)]
pub struct Problems {
    /// Bumped by whatever changed the list (Clear Problems).
    epoch: AtomicU64,
    reading: AtomicBool,
    list: Mutex<Option<(u64, Arc<Vec<Problem>>)>>,
    selected: Mutex<Option<usize>>,
}

impl Problems {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Problems> {
        editor.services.get_or_init(Problems::default)
    }

    /// The list when it is there; reads it when what it depends on moved.
    #[must_use]
    pub fn get(editor: &Arc<Editor>) -> Option<Arc<Vec<Problem>>> {
        let this = Self::of(editor);
        let key = key(editor, &this);
        let held = this.list.lock().clone();
        if held.as_ref().is_some_and(|(k, _)| *k == key) {
            return held.map(|(_, l)| l);
        }
        if !this.reading.swap(true, Ordering::AcqRel) {
            let ed = editor.clone();
            editor.exec.spawn(async move {
                let list = read(&ed).await;
                let this = Self::of(&ed);
                *this.list.lock() = Some((key, Arc::new(list)));
                this.reading.store(false, Ordering::Release);
                ed.exec.wake().ping();
            });
        }
        // the last list while the next is read
        held.map(|(_, l)| l)
    }
}

/// What the list depends on: the project, the last build of the level that ended, a clear.
fn key(editor: &Editor, this: &Problems) -> u64 {
    let last_build = editor
        .jobs
        .snapshot()
        .iter()
        .filter(|j| !j.running() && j.group.as_deref().is_some_and(|g| g == gz_levelbuild::JOB_BUILD))
        .map(|j| j.id.0)
        .max()
        .unwrap_or(0);
    let rev = editor.project_rev().0;
    rev.wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ last_build.wrapping_mul(31)
        ^ this.epoch.load(Ordering::Acquire) << 48
}

async fn read(editor: &Arc<Editor>) -> Vec<Problem> {
    let mut a = Args::new();
    a.insert("limit".into(), json!(10_000));
    let reply = editor.call("level.problems", a, editor.ui_origin()).await;
    reply
        .get("problems")
        .and_then(Value::as_array)
        .map(|l| l.iter().map(Problem::of).collect())
        .unwrap_or_default()
}

/// The list changed by something other than a build: it is read again.
pub fn changed(editor: &Editor) {
    Problems::of(editor).epoch.fetch_add(1, Ordering::AcqRel);
    editor.exec.wake().ping();
}

/// Selects a problem's object and frames it, or frames its place, off the UI thread.
pub fn go_to(editor: &Arc<Editor>, p: &Problem) {
    let ed = editor.clone();
    let p = p.clone();
    editor.exec.spawn(async move { go(&ed, &p).await });
}

/// Selects a problem's object and frames it, or frames its place.
pub async fn go(ed: &Arc<Editor>, p: &Problem) {
    {
        if let Some(name) = &p.object {
            let mut a = Args::new();
            a.insert("objects".into(), json!([name]));
            let reply = ed.call("scene.select", a, ed.ui_origin()).await;
            if reply.get("ok").and_then(Value::as_bool) == Some(true)
                && reply.get("selected").and_then(Value::as_u64).unwrap_or(0) > 0
            {
                let mut a = Args::new();
                a.insert("action".into(), json!("frame"));
                a.insert("scope".into(), json!("selection"));
                ed.call("viewport.camera", a, ed.ui_origin()).await;
                return;
            }
        }
        if let Some(at) = p.position {
            let c = glam::Vec3::from(at);
            let reach = glam::Vec3::splat(4.0);
            let b = gz_render::Aabb { min: c - reach, max: c + reach };
            let vp = gz_ui_viewport::Viewport::of(ed);
            vp.with(|s| {
                let size = s.size;
                s.nav.zoom_extents(&b, size);
            });
            ed.exec.wake().ping();
        }
    }
}

/// Marks a row of the panel as the one gone to.
pub fn select(editor: &Editor, row: usize) {
    *Problems::of(editor).selected.lock() = Some(row);
    editor.exec.wake().ping();
}

/// What `level.show` says of the panel.
#[must_use]
pub fn state_json(editor: &Arc<Editor>) -> Value {
    let this = Problems::of(editor);
    let list = this.list.lock().clone().map(|(_, l)| l);
    json!({
        "rows": list.as_ref().map_or(0, |l| l.len()),
        "selected": *this.selected.lock(),
    })
}

const COLUMNS: [Col<'static>; 3] =
    [Col::new("Problem", 360.0), Col::new("Object", 160.0), Col::new("Step", 100.0)];

/// Draws the panel.
pub fn draw(ui: &mut Ui, editor: &Arc<Editor>) {
    let Some(list) = Problems::get(editor) else {
        state::busy_line(ui, "Reading the problems...");
        return;
    };
    if list.is_empty() {
        state::empty_state(
            ui,
            Icon::Success,
            "No problems",
            "What a build of the level finds is listed here, each on the object it is about.",
            None,
        );
        return;
    }
    let this = Problems::of(editor);
    let selected = *this.selected.lock();
    let p = palette(ui.ctx());
    let ev =
        Table::new("level_problems", &COLUMNS, list.len()).selected(selected).show(ui, |ui, row, col| {
            let Some(it) = list.get(row) else { return };
            match col {
                0 => {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let tone = if it.error { Tone::Error } else { Tone::Warning };
                        let icon = if it.error { Icon::Error } else { Icon::Warning };
                        let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                        gz_ui::icons::draw(ui.painter(), icon, r, p.tone(tone));
                        gz_ui::text::label(ui, Tx::ui(&it.text), Font::Body, p.text, Clip::End)
                            .tip(Tx::ui(&it.text));
                    });
                }
                1 => {
                    let name = it.object.as_deref().unwrap_or("");
                    gz_ui::text::label(ui, Tx::raw(name), Font::Body, p.text_secondary, Clip::Start);
                }
                _ => {
                    let step = gz_levelbuild::Step::ORDER
                        .into_iter()
                        .find(|s| s.title() == it.step)
                        .map_or_else(|| Tx::raw(it.step.as_str()), |s| crate::menus::step_tx(s.title()));
                    gz_ui::text::label(ui, step, Font::Small, p.text_muted, Clip::End);
                }
            }
        });
    if let Some(row) = ev.clicked.or(ev.double_clicked) {
        *this.selected.lock() = Some(row);
        if let Some(it) = list.get(row) {
            go_to(editor, it);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_reads_its_step_by_title_and_its_place() {
        let v = json!({ "step": "geometry", "code": "E1", "severity": "error", "text": "A face is refused.",
            "object": "wall_01", "position": [1.0, 2.0, 3.0] });
        let p = Problem::of(&v);
        assert_eq!(p.step, "Geometry");
        assert!(p.error);
        assert_eq!(p.object.as_deref(), Some("wall_01"));
        assert_eq!(p.position, Some([1.0, 2.0, 3.0]));
        let w = Problem::of(&json!({ "step": "spawn", "severity": "warning", "text": "x" }));
        assert!(!w.error && w.object.is_none() && w.position.is_none());
    }
}
