//! Recover Unsaved Scene: the M page card that takes the Start page over while the open
//! project offers a snapshot of a session that did not close cleanly. Restore (the default) and Discard... - which asks
//! first, naming the snapshot's time and the older snapshots - are `gz-scene`'s `recovery`
//! command.
//!
//! The recovery's state is read on the pools (it looks at files), never in a frame: the card
//! reads what the last look found.

use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::Ui;
use gz_app::{DocCx, PageCard, ShellBuilder};
use gz_editor::{Args, Editor, SmolStr, Value};
use gz_scene::SceneService;
use gz_scene::recovery::{Recovery, Status, read_manifest};
use gz_ui::text::Clip;
use gz_ui::{Answer, CardAction, ConfirmSpec, Font, Kind, Tone, card, palette};
use parking_lot::Mutex;

use crate::tools::asker;

/// What the last look found.
#[derive(Clone, Debug, Default)]
struct Seen {
    status: Option<Status>,
    /// How many snapshots the manifest keeps.
    snapshots: usize,
    error: String,
}

#[derive(Default)]
struct RecoveryView {
    seen: Mutex<Seen>,
    /// When the last look started, and for which project.
    asked: Mutex<Option<(Instant, std::path::PathBuf)>>,
}

/// The last look, and another one started when it is half a second old.
fn seen(editor: &Editor) -> Seen {
    let view = editor.services.get_or_init(RecoveryView::default);
    let Some(session) = editor.project_opt() else {
        *view.seen.lock() = Seen::default();
        return Seen::default();
    };
    let Some(svc) = session.services.get::<SceneService>() else { return Seen::default() };
    {
        let offered = view.seen.lock().status.as_ref().is_none_or(|s| s.snapshot_available);
        // often while a snapshot is on offer (or nothing is known yet), rarely after
        let every = if offered { Duration::from_millis(500) } else { Duration::from_secs(5) };
        let mut asked = view.asked.lock();
        let due = asked.as_ref().is_none_or(|(t, root)| *root != svc.root || t.elapsed() >= every);
        if due && let Ok((_, project)) = gz_project::Project::of(editor) {
            *asked = Some((Instant::now(), svc.root.clone()));
            let rec = svc.recovery.clone();
            let dirty = svc.store.is_dirty();
            let root = svc.root.clone();
            let v = view.clone();
            let wake = editor.exec.wake().clone();
            editor.exec.spawn_io(move || {
                rec.bind(&root);
                let linked = project.game_linked();
                let status = rec.status(dirty, linked);
                let snapshots = std::fs::read_to_string(Recovery::dir_of(&root).join("manifest.ltx"))
                    .map(|t| read_manifest(&t).len())
                    .unwrap_or(0);
                let mut s = v.seen.lock();
                let changed = s.status.as_ref() != Some(&status) || s.snapshots != snapshots;
                s.status = Some(status);
                s.snapshots = snapshots;
                drop(s);
                if changed {
                    wake.ping();
                }
            });
        }
    }
    view.seen.lock().clone()
}

/// The card wants the page: a snapshot is on offer.
fn wants(editor: &Editor) -> bool {
    seen(editor).status.is_some_and(|s| s.snapshot_available && s.can_discard)
}

/// A FILETIME as local `YYYY-MM-DD HH:MM:SS`.
#[must_use]
pub fn format_saved_at(filetime: u64) -> String {
    // 100 ns ticks since 1601-01-01
    const EPOCH_DIFF: u64 = 11_644_473_600;
    let secs = (filetime / 10_000_000).saturating_sub(EPOCH_DIFF) as i64;
    let Ok(utc) = time::OffsetDateTime::from_unix_timestamp(secs) else { return "unknown".to_owned() };
    let local = time::UtcOffset::current_local_offset().map_or(utc, |o| utc.to_offset(o));
    let f = time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]:[second]");
    local.format(&f).unwrap_or_else(|_| "unknown".to_owned())
}

fn post(editor: &Arc<Editor>, action: &str) {
    let ed = editor.clone();
    let action = action.to_owned();
    editor.exec.spawn(async move {
        let mut args = Args::new();
        args.insert("action".into(), Value::String(action));
        let reply = ed.call("scene.recover", args, ed.ui_origin()).await;
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            let e = reply
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("The recovery did not answer.")
                .to_owned();
            let view = ed.services.get_or_init(RecoveryView::default);
            view.seen.lock().error = e;
        }
        *ed.services.get_or_init(RecoveryView::default).asked.lock() = None;
        ed.exec.wake().ping();
    });
}

fn draw(ui: &mut Ui, cx: &mut DocCx<'_>) {
    let editor = cx.editor.clone();
    let s = seen(&editor);
    let Some(st) = s.status.clone() else { return };
    let p = palette(ui.ctx());
    let saved_at = format_saved_at(st.saved_at);
    let scene = if st.source.is_empty() { "Untitled scene".to_owned() } else { st.source.clone() };
    let mut clicked = None;
    card::page(ui, gz_app::contract::WINDOW_START_PAGE, |ui| {
        card::page_card(ui, "recovery", "Recover Unsaved Scene", 560.0, |ui| {
            gz_ui::text::label(
                ui,
                "The previous session did not close cleanly.",
                Font::Strong,
                p.text,
                Clip::End,
            );
            gz_ui::text::prose(
                ui,
                "A snapshot of its unsaved scene was kept. Restore opens it as an unsaved scene; the scene file stays as it is.",
            );
            ui.add_space(8.0);
            for (k, v, clip) in
                [("Saved", saved_at.as_str(), Clip::End), ("Scene", scene.as_str(), Clip::Start)]
            {
                ui.horizontal(|ui| {
                    ui.allocate_ui(egui::Vec2::new(gz_ui::units::em(5.0), gz_ui::metrics::ROW), |ui| {
                        gz_ui::text::label(ui, k, Font::Body, p.text_secondary, Clip::End)
                    });
                    gz_ui::text::label(ui, v, Font::Body, p.text, clip);
                });
            }
            if st.source_newer {
                ui.add_space(4.0);
                gz_ui::text::status(
                    ui,
                    Tone::Warning,
                    "The scene file changed after this snapshot. Restore only if you need the older unsaved state.",
                );
            }
            let error = if s.error.is_empty() { st.last_error.clone() } else { s.error.clone() };
            if !error.is_empty() {
                ui.add_space(4.0);
                gz_ui::text::status(ui, Tone::Error, &error);
            }
            let restore_why = if st.dirty {
                "Save or discard the open scene first."
            } else {
                "The recovery cannot restore now."
            };
            let actions = [
                CardAction::new("Discard...", Kind::GhostDanger)
                    .enabled(st.can_discard, "The recovery cannot discard now."),
                CardAction::new("Restore", Kind::Primary)
                    .default_action()
                    .enabled(st.can_restore, restore_why),
            ];
            clicked = card::card_footer(ui, "", Tone::Muted, &actions);
        });
    });
    match clicked {
        Some(1) => post(&editor, "restore"),
        Some(0) => {
            let mut items = vec![format!("The unsaved changes to {scene}")];
            if s.snapshots > 1 {
                let n = s.snapshots - 1;
                items.push(format!("{n} older {}", if n == 1 { "snapshot" } else { "snapshots" }));
            }
            let spec = ConfirmSpec {
                id: SmolStr::new("discard_recovery").to_string(),
                title: "Discard Recovery".into(),
                headline: format!("Discard the recovery snapshot saved at {saved_at}?"),
                total: items.len(),
                items,
                consequence: String::new(),
                action: "Discard".into(),
                destructive: true,
            };
            let ed = editor.clone();
            let line = gz_ui::dialog::confirm(ui.ctx(), asker(&editor), spec, move |answer| {
                if answer == Answer::Yes {
                    post(&ed, "discard");
                }
            });
            if let Some(line) = line {
                editor.log.plain(line);
            }
        }
        _ => {}
    }
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.page_card(PageCard { id: SmolStr::new("recovery"), wants: Box::new(wants), draw: Box::new(draw) });
}

#[cfg(test)]
mod tests {
    use super::format_saved_at;

    #[test]
    fn a_zero_time_is_still_a_date() {
        assert!(!format_saved_at(0).is_empty());
    }
}
