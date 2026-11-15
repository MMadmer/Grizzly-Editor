//! `screens.view`: opens or reports the Screens & Music tab - `open`, `surface`, `screen` /
//! `track` / `layer` / `playlist`, `show=streamer` (the selected track's Plays Instead list) - and
//! lands files at a window point the way a drop from Explorer does (`simulate_drop`, `x`, `y` in
//! pixels). Answers `{tab, open, active, surface, screen, track, playlist, layer, status}`, and
//! with a copyrighted track selected `picker` (`count`, the first `rows`); the data is
//! `screens.info` / `screens.set` / `screens.import`.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use gz_base::Origin;
use gz_editor::{Args, Editor, Reply};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::screens::ScreensTab;

fn arg(args: &Args, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn arg_bool(args: &Args, key: &str) -> bool {
    match args.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
        Some(Value::String(s)) => {
            let t = s.trim_start_matches(' ');
            t.starts_with("true") || t.starts_with('1')
        }
        _ => false,
    }
}

fn arg_f32(args: &Args, key: &str) -> f32 {
    match args.get(key) {
        Some(Value::Number(n)) => n.as_f64().map_or(0.0, |v| v as f32),
        Some(Value::String(s)) => crate::gamefile::atof(s),
        _ => 0.0,
    }
}

fn arg_int(args: &Args, key: &str, def: i64) -> i64 {
    match args.get(key) {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|v| v as i64)).unwrap_or(def),
        Some(Value::String(s)) => {
            let t = s.trim_start_matches([' ', '\t']);
            if t.starts_with(|c: char| c == '-' || c.is_ascii_digit()) {
                crate::gamefile::atoi(t)
            } else {
                def
            }
        }
        _ => def,
    }
}

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ViewParams {
    /// Brings the tab up.
    #[serde(default)]
    open: bool,
    /// The surface the tab shows: loading or menu.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    surface: Option<String>,
    /// Selects a screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    screen: Option<String>,
    /// With `screen`: selects its layer (0 the bottom one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layer: Option<i64>,
    /// Selects a track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    track: Option<String>,
    /// Selects a playlist: its card shows on the right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    playlist: Option<String>,
    /// streamer: puts up the Plays Instead list of the selected track (a copyrighted one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show: Option<String>,
    /// Files (separated by |) dropped at the window point `x`, `y` (px), the way a drop from
    /// Explorer lands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    simulate_drop: Option<String>,
    /// The window point of `simulate_drop`, from the left, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    x: Option<f32>,
    /// The window point of `simulate_drop`, from the top, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    y: Option<f32>,
}

pub async fn screens_view(editor: Arc<Editor>, p: ViewParams, _origin: Origin) -> Reply {
    let args = match serde_json::to_value(&p) {
        Ok(Value::Object(map)) => map,
        _ => Args::new(),
    };
    let session = editor.project_opt();
    let tab = session.as_ref().map(|s| ScreensTab::of_session(s));
    if arg_bool(&args, "open") && tab.is_some() {
        let mut a = Args::new();
        a.insert("action".into(), json!("open"));
        a.insert("kind".into(), json!("screens"));
        let _ = editor.call("ui.tab", a, Origin::Mcp).await;
    }
    if let Some(tab) = &tab {
        let mut v = tab.v.lock();
        // a view command puts up what it asks for: a list the last one left up goes
        if p.show.is_none() {
            v.close_picker = Some(v.sel_track.clone());
        }
        if let Some(s) = arg(&args, "surface") {
            v.surface = usize::from(s == "menu");
        }
        if let Some(s) = arg(&args, "screen") {
            v.sel_screen = s;
            v.sel_track.clear();
            v.sel_layer = arg_int(&args, "layer", -1);
        }
        if let Some(t) = arg(&args, "track") {
            v.sel_track = t;
            v.sel_screen.clear();
            v.sel_playlist.clear();
            v.sel_layer = -1;
        }
        if let Some(pl) = arg(&args, "playlist") {
            v.sel_playlist = pl;
            v.sel_screen.clear();
            v.sel_track.clear();
            v.sel_layer = -1;
        }
    }
    let show = arg(&args, "show");
    if let Some(what) = show.as_deref()
        && what != "streamer"
    {
        return Err(gz_editor::CmdError::invalid(format!("show takes streamer, not '{what}'")));
    }
    // the same way a drop from Explorer lands: at a point of the editor's window
    if let Some(list) = arg(&args, "simulate_drop") {
        let files: Vec<std::path::PathBuf> = list
            .split('|')
            .map(|f| f.trim_matches([' ', '\t', '\r', '\n']))
            .filter(|f| !f.is_empty())
            .map(std::path::PathBuf::from)
            .collect();
        let (x, y) = (arg_f32(&args, "x"), arg_f32(&args, "y"));
        let landed = gz_app::on_ui(&editor, move |_, ctx| {
            let ppp = ctx.pixels_per_point();
            gz_ui::drop::simulate_files(ctx, files, egui::pos2(x / ppp, y / ppp));
            Ok(())
        })
        .await;
        // the frames that take the drop, then the import and the save it started
        if landed.is_ok() {
            let _ = editor.frames.after(2).await;
            if let Some(tab) = &tab {
                let deadline = Instant::now() + Duration::from_secs(60);
                while tab.busy.load(Ordering::SeqCst) > 0 && Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }
    }
    let v = tab.as_ref().map(|t| t.v.lock().clone()).unwrap_or_default();
    let picker = picker(&editor, &v.sel_track).await;
    if show.is_some() {
        if picker.is_null() {
            return Err(gz_editor::CmdError::new(
                "refused",
                "show=streamer puts up the Plays Instead list of a copyrighted track: select one (track=) first",
            ));
        }
        if let Some(tab) = &tab {
            tab.v.lock().open_picker = true;
        }
        let _ = editor.frames.after(2).await;
    }
    let active = gz_app::on_ui(&editor, |shell, _| Ok(shell.active() == "screens")).await.unwrap_or(false);
    let v = tab.as_ref().map(|t| t.v.lock().clone()).unwrap_or(v);
    Ok(json!({
        "tab": "screens",
        "open": v.open,
        "active": active,
        "surface": if v.surface == 1 { "menu" } else { "loading" },
        "screen": v.sel_screen,
        "track": v.sel_track,
        "playlist": v.sel_playlist,
        "layer": if tab.is_some() { v.sel_layer } else { -1 },
        "status": v.status,
        "picker": picker,
    }))
}

/// The most rows of the Plays Instead list an answer names.
const PICKER_ROWS: usize = 200;

/// The Plays Instead list of the track `id` as the tab offers it, when it is a copyrighted track
/// of the module: how many rows, the first ones (`id`, `from`); `null` otherwise.
async fn picker(editor: &Arc<Editor>, id: &str) -> Value {
    if id.is_empty() {
        return Value::Null;
    }
    let Ok(manifest) = gz_mod::ManifestService::of(editor).await else { return Value::Null };
    let _ = manifest.sync().await;
    let snap = manifest.snapshot();
    let Some(doc) = snap.doc() else { return Value::Null };
    let s = gz_screens::Screens::parse(doc);
    if !s.track(id).is_some_and(|t| t.copyrighted) {
        return Value::Null;
    }
    let sounds = gz_screens::Sounds::now(editor).await;
    let free = editor.exec.io(move || sounds.free()).await;
    let rows = crate::screens::choices(&s, id, &free);
    let first: Vec<Value> =
        rows.iter().take(PICKER_ROWS).map(|r| json!({ "id": r.id, "from": r.from.name() })).collect();
    let tracks: Vec<&str> =
        rows.iter().filter(|r| r.from == crate::screens::ChoiceFrom::Track).map(|r| r.id.as_str()).collect();
    json!({ "count": rows.len(), "tracks": tracks, "rows": first })
}
