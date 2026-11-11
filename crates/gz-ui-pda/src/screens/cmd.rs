//! `screens_view` (`EditorScreens::McpView`, frozen wire): opens or reports the Screens & Music
//! tab - `open`, `surface`, `screen` / `track` / `layer` - and lands files at a window point the
//! way a drop from Explorer does (`simulate_drop`, `x`, `y` in pixels). Answers `{tab, open,
//! active, surface, screen, track, layer, status}`; the data path stays `screens_get` /
//! `screens_set`.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use gz_base::Origin;
use gz_editor::{Args, Editor, Reply};
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

pub async fn screens_view(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let session = editor.project_opt();
    let tab = session.as_ref().map(|s| ScreensTab::of_session(s));
    if arg_bool(&args, "open") && tab.is_some() {
        let mut a = Args::new();
        a.insert("action".into(), json!("open"));
        a.insert("kind".into(), json!("screens"));
        let _ = editor.call("ui_tab", a, Origin::Mcp).await;
    }
    if let Some(tab) = &tab {
        let mut v = tab.v.lock();
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
            v.sel_layer = -1;
        }
    }
    // the same way a drop from Explorer lands: at a point of the editor's window
    if let Some(list) = arg(&args, "simulate_drop") {
        let files: Vec<String> = list
            .split('|')
            .map(|f| f.trim_matches([' ', '\t', '\r', '\n']))
            .filter(|f| !f.is_empty())
            .map(str::to_owned)
            .collect();
        let (x, y) = (arg_f32(&args, "x"), arg_f32(&args, "y"));
        let landed = gz_app::on_ui(&editor, move |_, ctx| {
            let ppp = ctx.pixels_per_point();
            crate::drop::simulate_drop(ctx, files, egui::pos2(x / ppp, y / ppp));
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
    let active = gz_app::on_ui(&editor, |shell, _| Ok(shell.active() == "screens")).await.unwrap_or(false);
    let v = tab.as_ref().map(|t| t.v.lock().clone()).unwrap_or_default();
    Ok(json!({
        "tab": "screens",
        "open": v.open,
        "active": active,
        "surface": if v.surface == 1 { "menu" } else { "loading" },
        "screen": v.sel_screen,
        "track": v.sel_track,
        "layer": if tab.is_some() { v.sel_layer } else { -1 },
        "status": v.status,
    }))
}
