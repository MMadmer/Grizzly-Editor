//! The frame over MCP (docs/UI_DESIGN.md, section 7). The wire is the C++ editor's
//! (`LevelMcp.cpp`, `GrizzlyMCP.cpp`, `UITitleBar.cpp`): the same names, fields and texts.
//!
//! A command that needs the shell runs a closure on the UI thread ([`on_ui`]); one whose answer
//! describes a picture waits for fresh frames first.

use std::sync::Arc;
use std::time::Duration;

use egui::{Context, Rect, ViewportCommand};
use gz_editor::{Args, ArgsExt, CmdError, Editor, Origin, Registry, Reply, Value, json};
use gz_ui::dialog::{self, Asker};
use gz_ui::{Tone, audit, layout};
use serde_json::Map;

use crate::contract;
use crate::cpp::{CommandVar, CppAction};
use crate::doc::DocCx;
use crate::doc::{Drawer, OpenRequest, PanelOp};
use crate::keys::Chord;
use crate::palette as pal;
use crate::queue::{UiQueue, on_ui};
use crate::scroll::{self, ScrollTo};
use crate::settings::{
    self, EditorSettings, PREFS, PrefType, WIRE_PREFS, parse_pref, pref_json, wire_pref_def,
};
use crate::shell::{CloseMode, OpenError, ShellState, default_log_state};

const AREA: &str = "shell";

fn invalid(text: impl Into<String>) -> CmdError {
    CmdError::invalid(text)
}

fn r0(v: f32) -> i64 {
    v.round() as i64
}

fn r2(v: f32) -> f64 {
    (f64::from(v) * 100.0).round() / 100.0
}

fn r3(v: f32) -> f64 {
    (f64::from(v) * 1000.0).round() / 1000.0
}

/// x, y, w, h in px of a rect in pt.
fn rect_px(r: Rect, ppp: f32) -> Value {
    json!([r0(r.min.x * ppp), r0(r.min.y * ppp), r0(r.width() * ppp), r0(r.height() * ppp)])
}

fn str_arg<'a>(args: &'a Args, key: &str) -> &'a str {
    args.str(key).unwrap_or("")
}

/// A value given as a string or a number, as text.
fn text_arg(args: &Args, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn merge(mut out: Map<String, Value>, more: Value) -> Value {
    if let Value::Object(m) = more {
        out.extend(m);
    }
    Value::Object(out)
}

/// The tab that is up, as an opener answers.
fn answer_active(shell: &ShellState) -> Value {
    let mut out = Map::new();
    if let Some(tab) = shell.tab_json(shell.active()) {
        out.insert("tab".into(), tab);
    }
    Value::Object(out)
}

fn tabs_fields(shell: &ShellState) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("active_tab".into(), json!(shell.active()));
    out.insert("tabs".into(), Value::Array(shell.tabs_json()));
    out
}

fn dirty_json(list: &[(String, String)]) -> Value {
    Value::Array(list.iter().map(|(id, label)| json!({ "id": id, "label": label })).collect())
}

fn unsaved(list: &[(String, String)]) -> CmdError {
    CmdError::unsaved(dirty_json(list))
        .with_data(json!({ "dirty": dirty_json(list), "note": "pass save=true or discard=true" }))
}

/// The modal on top: a kit dialog, else the palette. `(title, id, rect pt)`.
fn top_modal(shell: &ShellState, ctx: &Context) -> Option<(String, String, Rect)> {
    if let Some(d) = dialog::modal_on_top(ctx) {
        let ppp = ctx.pixels_per_point();
        let r = Rect::from_min_max(
            egui::pos2(d.rect_px[0] / ppp, d.rect_px[1] / ppp),
            egui::pos2(d.rect_px[2] / ppp, d.rect_px[3] / ppp),
        );
        return Some((d.title, d.id, r));
    }
    shell
        .palette_open()
        .then(|| ("Command Palette".to_owned(), contract::PALETTE_ID.to_owned(), shell.palette_rect()))
}

/// The MCP id a kit dialog answers to: the contract's id for one of its dialogs, else
/// `dlg.<name>`.
fn dialog_mcp_id(kit: &str) -> String {
    contract::DIALOGS
        .iter()
        .find(|(_, w)| w.ends_with(&format!("###{kit}")))
        .map_or_else(|| kit.to_owned(), |(id, _)| (*id).to_owned())
}

fn dialogs_json(shell: &ShellState, ctx: &Context) -> Vec<Value> {
    let ppp = ctx.pixels_per_point();
    let mut out: Vec<Value> = dialog::open_dialogs(ctx)
        .into_iter()
        .map(|d| {
            let r = Rect::from_min_max(egui::pos2(d.rect_px[0] / ppp, d.rect_px[1] / ppp), egui::pos2(d.rect_px[2] / ppp, d.rect_px[3] / ppp));
            json!({ "id": dialog_mcp_id(&d.id), "title": d.title, "window": format!("{}###{}", d.title, d.id), "rect": rect_px(r, ppp) })
        })
        .collect();
    if shell.palette_open() {
        out.push(json!({ "id": contract::PALETTE_ID, "title": "Command Palette", "window": contract::PALETTE_WINDOW, "rect": rect_px(shell.palette_rect(), ppp) }));
    }
    out
}

// ---- state ------------------------------------------------------------------------------------

async fn state(editor: Arc<Editor>, _args: Args, _origin: Origin) -> Reply {
    let mut out = Map::new();
    let info = crate::project::project_info(&editor);
    if let Value::Object(p) = crate::project::state_fields(&info) {
        out.extend(p);
    }
    let linked = info.game_linked;
    let ui = match UiQueue::of(&editor) {
        Ok(q) => Some(
            q.run(|shell, ctx| {
                let level = shell.tab_json(crate::tabs::LEVEL);
                let scene = level.as_ref().and_then(|t| t.get("path")).and_then(Value::as_str).unwrap_or("").replace('\\', "/");
                let mut m = Map::new();
                m.insert("scene".into(), json!(scene));
                m.insert("fps".into(), json!(shell.fps.round() as i64));
                m.insert("active_tab".into(), json!(shell.active()));
                let tabs: Vec<Value> = shell
                    .tabs_json()
                    .into_iter()
                    .map(|t| json!({ "id": t["id"], "kind": t["kind"], "label": t["label"], "path": t["path"], "dirty": t["dirty"], "active": t["active"], "missing": t["missing"] }))
                    .collect();
                m.insert("tabs".into(), Value::Array(tabs));
                m.insert("dialog".into(), top_modal(shell, ctx).map_or(Value::Null, |(title, id, _)| json!({ "title": title, "id": id })));
                m.insert("ui_scale".into(), json!(r2(ctx.pixels_per_point())));
                Ok(m)
            })
            .await?,
        ),
        Err(_) => None,
    };
    match ui {
        Some(mut m) => {
            out.insert("scene".into(), m.remove("scene").unwrap_or(json!("")));
            out.insert("fps".into(), m.remove("fps").unwrap_or(json!(0)));
            out.insert("active_tab".into(), m.remove("active_tab").unwrap_or(json!("")));
            out.insert("tabs".into(), m.remove("tabs").unwrap_or(json!([])));
            out.insert("dialog".into(), m.remove("dialog").unwrap_or(Value::Null));
            out.insert("start_page".into(), json!(!(editor.project_opt().is_some() && linked)));
            out.insert("ui_scale".into(), m.remove("ui_scale").unwrap_or(json!(1.0)));
        }
        None => {
            out.insert("scene".into(), json!(""));
            out.insert("fps".into(), json!(0));
            out.insert("active_tab".into(), json!(""));
            out.insert("tabs".into(), json!([]));
            out.insert("dialog".into(), Value::Null);
            out.insert("start_page".into(), json!(!(editor.project_opt().is_some() && linked)));
            out.insert("ui_scale".into(), json!(1.0));
        }
    }
    Ok(Value::Object(out))
}

// ---- ui_list ----------------------------------------------------------------------------------

fn panel_json(shell: &ShellState, ctx: &Context, id: &str, window: &str) -> Value {
    let ppp = ctx.pixels_per_point();
    let info = shell.level_panel(id).unwrap_or_default();
    let on_level = shell.active() == crate::tabs::LEVEL;
    let visible = on_level && info.visible;
    let drawer = shell.panel_drawer(id);
    let in_drawer = drawer.is_some() && drawer == shell.drawer;
    json!({
        "id": id,
        "window": window,
        "open": info.open || id == "viewport" && shell.level_panel(id).is_none_or(|p| p.open),
        "visible": visible,
        "docked": info.docked,
        "floating": info.floating,
        "focused": info.focused,
        "closable": id != "viewport",
        "host": if info.floating && info.open { "window" } else if visible { "panel" } else if in_drawer { "drawer" } else { "none" },
        "rect": match (visible, info.rect) {
            (true, Some([x, y, w, h])) => json!([r0(x), r0(y), r0(w), r0(h)]),
            _ => { let _ = ppp; Value::Null }
        },
    })
}

async fn ui_list(editor: Arc<Editor>, _args: Args, _origin: Origin) -> Reply {
    editor.frames.after(2).await?;
    on_ui(&editor, |shell, ctx| {
        let ppp = ctx.pixels_per_point();
        let mut out = tabs_fields(shell);
        out.insert(
            "panels".into(),
            Value::Array(contract::PANELS.iter().map(|(id, w)| panel_json(shell, ctx, id, w)).collect()),
        );
        out.insert("drawer".into(), json!(shell.drawer.map_or("none", Drawer::id)));
        let drawers: Vec<Value> = contract::DRAWERS
            .iter()
            .map(
                |(id, w)| json!({ "id": id, "window": w, "open": shell.drawer.map(Drawer::id) == Some(*id) }),
            )
            .collect();
        out.insert("drawers".into(), Value::Array(drawers));
        out.insert("dialogs".into(), Value::Array(dialogs_json(shell, ctx)));
        let popups: Vec<Value> = shell
            .popups()
            .iter()
            .map(
                |p| json!({ "id": p.id, "title": p.title, "window": p.window, "rect": rect_px(p.rect, ppp) }),
            )
            .collect();
        out.insert("popups".into(), Value::Array(popups));
        out.insert("work_area".into(), rect_px(shell.work(), ppp));
        out.insert("start_page".into(), json!(shell.start_page()));
        out.insert("size_class".into(), json!(shell.size_class()));
        let level = shell.level_state();
        out.insert(
            "layout_custom".into(),
            json!(level.get("layout_custom").and_then(Value::as_bool).unwrap_or(false)),
        );
        out.insert(
            "viewport_maximized".into(),
            json!(level.get("viewport_maximized").and_then(Value::as_bool).unwrap_or(false)),
        );
        out.insert("ui_scale".into(), json!(r2(ppp)));
        out.insert("dpi".into(), json!(r3(ppp)));
        Ok(Value::Object(out))
    })
    .await
}

// ---- ui_tab -----------------------------------------------------------------------------------

async fn ui_tab(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let action = args.str_or("action", "list").to_owned();
    let id = str_arg(&args, "id").to_owned();
    let kind = str_arg(&args, "kind").to_owned();
    let path = str_arg(&args, "path").to_owned();
    let source = str_arg(&args, "source").to_owned();
    let save = args.flag("save");
    let discard = args.flag("discard");
    let has_project = editor.project_opt().is_some();
    if action == "close" && save && !discard {
        // the tab's save, waited for until it is clean: a document that writes on the pools has
        // not finished when its save returns
        let wanted = id.clone();
        let target = on_ui(&editor, move |shell, _| {
            Ok(if wanted.is_empty() { shell.active().to_owned() } else { wanted })
        })
        .await?;
        crate::unsaved::save_tabs(&editor, vec![target]).await.map_err(invalid)?;
    }
    on_ui(&editor, move |shell, ctx| match action.as_str() {
        "list" => Ok(Value::Object(tabs_fields(shell))),
        "next" | "prev" => {
            shell.cycle(ctx, if action == "next" { 1 } else { -1 });
            Ok(answer_active(shell))
        }
        "activate" => {
            if id.is_empty() || shell.tab_json(&id).is_none() {
                return Err(invalid(format!("no tab '{id}' (ui_tab action=list names them)")));
            }
            if !shell.activate(ctx, &id) {
                return Err(invalid("play in editor holds the Level tab"));
            }
            Ok(answer_active(shell))
        }
        "open" => {
            if !id.is_empty() && shell.tab_json(&id).is_some() {
                if !shell.activate(ctx, &id) {
                    return Err(invalid("play in editor holds the Level tab"));
                }
                return Ok(answer_active(shell));
            }
            // "quest:<rel>" and the singletons' ids name what to open
            let (mut kind, mut path) = (kind, path);
            if !id.is_empty() {
                match id.split_once(':') {
                    Some((k, p)) => {
                        if kind.is_empty() {
                            kind = k.to_owned();
                        }
                        if path.is_empty() {
                            path = p.to_owned();
                        }
                    }
                    None if kind.is_empty() => kind.clone_from(&id),
                    None => {}
                }
            }
            if kind.is_empty() {
                return Err(invalid("pass id=, or kind= (and path= for a file)"));
            }
            match kind.as_str() {
                "quest" | "beh" if path.is_empty() => {
                    return Err(invalid("path= names the graph (project-relative)"));
                }
                "model" if path.is_empty() => {
                    return Err(invalid("path= names the model (a visual, meshes-relative)"));
                }
                "texture"
                    if !matches!(source.as_str(), "" | "project" | "0" | "editor" | "1" | "darf" | "2") =>
                {
                    return Err(invalid("source must be project, editor or darf"));
                }
                "mod_settings" | "releases" if !has_project => return Err(invalid("no project is open")),
                _ => {}
            }
            let req = OpenRequest {
                kind,
                path: (!path.is_empty()).then_some(path),
                id: None,
                arg: (!source.is_empty()).then_some(source),
                activate: true,
            };
            if save && !shell.active().is_empty() {
                // a dirty singleton that switches target: saved first when asked
                let target = crate::doc::tab_id(&req.kind, req.path.as_deref()).to_string();
                if shell.tab_json(&target).is_some() {
                    shell.save_tab(ctx, &target).map_err(invalid)?;
                }
            }
            match shell.open(ctx, req, discard, Asker::Mcp) {
                Ok(()) => Ok(answer_active(shell)),
                Err(OpenError::Unsaved { id, label }) => Err(unsaved(&[(id, label)])),
                Err(OpenError::Other(e)) => Err(invalid(e)),
            }
        }
        "close" => {
            let id = if id.is_empty() { shell.active().to_owned() } else { id };
            if save && discard {
                return Err(invalid("save and discard exclude each other"));
            }
            let Some(info) = shell.tab_json(&id) else {
                return Err(invalid(format!("no tab '{id}' (ui_tab action=list names them)")));
            };
            let mode = if save {
                CloseMode::SaveFirst
            } else if discard {
                CloseMode::Discard
            } else {
                CloseMode::IfClean
            };
            match shell.close_tab(ctx, &id, mode) {
                Ok(()) => {
                    let mut out = Map::new();
                    out.insert("closed".into(), json!(id));
                    Ok(merge(out, Value::Object(tabs_fields(shell))))
                }
                Err(e) if e == "unsaved" => {
                    let label = info.get("label").and_then(Value::as_str).unwrap_or_default().to_owned();
                    Err(unsaved(&[(id, label)]))
                }
                Err(e) => Err(invalid(e)),
            }
        }
        _ => Err(invalid("action must be list, activate, open, close, next or prev")),
    })
    .await
}

// ---- ui_panel (until the Level package registers its own) -----------------------------------------

async fn ui_panel(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let id = str_arg(&args, "id").to_owned();
    let action = args.str_or("action", "focus").to_owned();
    // floating=true moves the panel into a window of its own, false docks it again
    let floating = args.bool("floating");
    let moved = floating.is_some();
    let answer = on_ui(&editor, move |shell, ctx| {
        if !shell.has_panel(&id) {
            return Err(invalid("id must be tools, world_outliner, details, level_settings, content_browser, output_log or viewport"));
        }
        let op = match (floating, action.as_str()) {
            (Some(true), _) => PanelOp::Float,
            (Some(false), _) => PanelOp::Dock,
            (None, "open") => PanelOp::Open,
            (None, "close") => PanelOp::Close,
            (None, "focus") => PanelOp::Focus,
            (None, "toggle") => PanelOp::Toggle,
            _ => return Err(invalid("action must be open, close, focus or toggle")),
        };
        if floating.is_some() && id == "viewport" {
            return Err(invalid("the viewport stays in the editor's window"));
        }
        if op == PanelOp::Float && !crate::shell::floating_panels(ctx) {
            return Err(invalid(
                "panels stay in the window: Preferences > General > Allow Panels Outside the Window is off (it takes a restart)",
            ));
        }
        let closing = op == PanelOp::Close || op == PanelOp::Toggle && shell.level_panel(&id).is_some_and(|p| p.open);
        if closing && id == "viewport" {
            return Err(invalid("the viewport stays on Level: F11 (View > Maximize Viewport) hides the panels"));
        }
        let host = shell.panel(ctx, &id, op).ok_or_else(|| invalid(format!("the {id} panel did not answer")))?;
        ctx.request_repaint();
        Ok((id, host))
    })
    .await?;
    // a window of its own is made after the frame that asked for it, and drawn in the next
    if moved {
        editor.frames.after(3).await?;
    }
    let (id, host) = answer;
    on_ui(&editor, move |shell, _| {
        let info = shell.level_panel(&id).unwrap_or_default();
        let host = if info.floating { "window" } else if host == "window" { "panel" } else { host };
        Ok(json!({ "id": id, "open": info.open, "host": host, "floating": info.floating, "active_tab": shell.active() }))
    })
    .await
}

// ---- ui_drawer ----------------------------------------------------------------------------------

async fn ui_drawer(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let action = str_arg(&args, "action").to_owned();
    let id = str_arg(&args, "id").to_owned();
    on_ui(&editor, move |shell, ctx| {
        let which = Drawer::parse(&id);
        let host = match action.as_str() {
            "open" => {
                let Some(which) = which else { return Err(invalid("id must be content or log")) };
                if !shell.has_drawer(which) {
                    return Err(invalid(format!("the {} drawer is not part of this build yet", which.id())));
                }
                if shell.drawer_panel_visible(which) {
                    shell.toggle_drawer(ctx, which);
                } else {
                    shell.open_drawer(which);
                }
                if shell.drawer == Some(which) { "drawer" } else { "panel" }
            }
            "close" => {
                if which.is_none() || shell.drawer == which {
                    shell.drawer = None;
                }
                "none"
            }
            _ => return Err(invalid("action must be open or close")),
        };
        Ok(json!({ "drawer": shell.drawer.map_or("none", Drawer::id), "host": host }))
    })
    .await
}

// ---- ui_dialog ----------------------------------------------------------------------------------

async fn ui_dialog(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let action = args.str_or("action", "list").to_owned();
    let id = str_arg(&args, "id").to_owned();
    on_ui(&editor, move |shell, ctx| {
        if action == "list" {
            let known: Vec<&str> = contract::DIALOGS.iter().map(|(i, _)| *i).collect();
            return Ok(json!({ "dialogs": dialogs_json(shell, ctx), "known": known }));
        }
        if id.is_empty() {
            return Err(invalid("id names the dialog (ui_contract lists them)"));
        }
        match action.as_str() {
            "open" => {
                shell.open_dialog(ctx, &id).map_err(invalid)?;
                let window = contract::window_of_known(&id).map_or_else(|| format!("###dlg.{id}"), str::to_owned);
                Ok(json!({ "id": id, "window": window, "note": "it comes up when the editor draws next: screenshot_editor names it as modal" }))
            }
            "close" => {
                if id == contract::PALETTE_ID && shell.palette_open() {
                    shell.close_palette();
                } else {
                    // a kit dialog goes by what follows ### in its window's name
                    let kit = match contract::window_of_known(&id).and_then(|w| w.split_once("###")) {
                        Some((_, name)) => name.to_owned(),
                        None if id.starts_with("dlg.") => id.clone(),
                        None => format!("dlg.{id}"),
                    };
                    if !dialog::close_dialog(ctx, &kit) {
                        return Err(invalid(format!("'{id}' is not open (ui_dialog action=list names what is)")));
                    }
                }
                Ok(json!({ "closing": id, "note": "it goes when the editor draws next" }))
            }
            _ => Err(invalid("action must be list, open or close")),
        }
    })
    .await
}

// ---- ui_popup, ui_toast -------------------------------------------------------------------------

async fn ui_popup(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let show = str_arg(&args, "show").to_owned();
    if show.is_empty() {
        return Err(invalid("show=<popup> or show=none (ui_contract lists the popups)"));
    }
    on_ui(&editor, move |shell, ctx| {
        shell.show_popup(ctx, &show).map_err(invalid)?;
        if show == "none" {
            return Ok(json!({ "popup": "none" }));
        }
        Ok(json!({ "popup": show, "note": "it comes up when its owner draws next; ui_popup show=none takes it down" }))
    })
    .await
}

async fn ui_toast(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let text = args.text("text").unwrap_or("A note.").to_owned();
    on_ui(&editor, move |_, ctx| {
        gz_ui::state::toast(ctx, Tone::Info, &text);
        Ok(Value::Null)
    })
    .await
}

// ---- ui_scroll, ui_split, ui_audit, ui_scale, ui_contract -----------------------------------------

async fn ui_scroll(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let id = str_arg(&args, "id").to_owned();
    let to = str_arg(&args, "to").to_owned();
    if id.is_empty() || to.is_empty() {
        return Err(invalid("ui_scroll takes id= and to=top|bottom|<pt>|item:<name>"));
    }
    on_ui(&editor, move |shell, ctx| {
        let window = shell.window_of(&id);
        if let Some(item) = to.strip_prefix("item:") {
            scroll::request_scroll(ctx, &window, ScrollTo::Item(item.to_owned()));
            ctx.request_repaint();
            return Ok(json!({ "id": id, "pending": true, "note": "a panel built on the kit scrolls to the item when it draws next" }));
        }
        let Some((y, max)) = scroll::last_scroll(ctx, &window) else { return Err(invalid(format!("'{id}' is not on screen"))) };
        let target = ScrollTo::parse(&to).ok_or_else(|| invalid("to must be top, bottom, a number of pt or item:<name>"))?;
        scroll::request_scroll(ctx, &window, target);
        ctx.request_repaint();
        let ppp = ctx.pixels_per_point();
        Ok(json!({
            "id": id,
            "window": window,
            "scroll_y": r0(y * ppp),
            "scroll_max_y": r0(max * ppp),
            "dpi": r3(ppp),
            "note": "px, as the last frame left them: it moves at the next",
        }))
    })
    .await
}

fn g(v: f32) -> Value {
    let s: f64 = format!("{v}").parse().unwrap_or(f64::from(v));
    if s.fract() == 0.0 { json!(s as i64) } else { json!(s) }
}

async fn ui_split(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let key = str_arg(&args, "key").to_owned();
    let value = args.f32("value");
    on_ui(&editor, move |_, ctx| {
        if key.is_empty() {
            let mut splits = Map::new();
            for (k, v) in layout::all(ctx) {
                splits.insert(k.to_string(), g(v));
            }
            return Ok(json!({ "splits": splits, "unit": "pt" }));
        }
        match value {
            Some(v) => layout::set(ctx, &key, v),
            None if !layout::has(ctx, &key) => {
                return Err(invalid(format!("no split '{key}' (ui_split with no key lists them)")));
            }
            None => {}
        }
        Ok(json!({ "key": key, "value": g(layout::get(ctx, &key, 0.0)), "unit": "pt" }))
    })
    .await
}

async fn ui_audit(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let id = str_arg(&args, "id").to_owned();
    on_ui(&editor, |_, ctx| {
        audit::begin(ctx);
        ctx.request_repaint();
        Ok(())
    })
    .await?;
    editor.frames.after(2).await?;
    on_ui(&editor, move |shell, ctx| {
        let entries = audit::take(ctx);
        let window = (!id.is_empty()).then(|| shell.window_of(&id));
        let ppp = ctx.pixels_per_point();
        let px = |r: [f32; 4]| json!([r0(r[0]), r0(r[1]), r0(r[2] - r[0]), r0(r[3] - r[1])]);
        let keep = |surface: &str| window.as_ref().is_none_or(|w| surface == w || surface.ends_with(w.as_str()));
        let (mut truncated, mut escaped, mut controls) = (Vec::new(), Vec::new(), Vec::new());
        for e in entries {
            match e {
                audit::AuditEntry::Truncated { surface, full, shown, rect } if keep(&surface) => {
                    truncated.push(json!({ "window": surface, "label": full, "shown": shown, "rect": px(rect) }));
                }
                audit::AuditEntry::Escaped { surface, what, rect } if keep(&surface) => {
                    escaped.push(json!({ "window": surface, "label": what, "by": "", "rect": px(rect) }));
                }
                audit::AuditEntry::Control { surface, label, enabled, why, help, rect } if keep(&surface) => {
                    controls.push(json!({ "window": surface, "label": label, "enabled": enabled, "why": why.unwrap_or_default(), "help": help.unwrap_or_default(), "rect": px(rect) }));
                }
                _ => {}
            }
        }
        Ok(json!({ "truncated": truncated, "escaped": escaped, "controls": controls, "dpi": r3(ppp) }))
    })
    .await
}

async fn ui_scale(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let value = args.f32("value");
    if let Some(v) = value
        && !(contract::UI_SCALE_MIN..=contract::UI_SCALE_MAX).contains(&v)
    {
        return Err(invalid(format!("value must be {}..{}", contract::UI_SCALE_MIN, contract::UI_SCALE_MAX)));
    }
    on_ui(&editor, move |_, ctx| {
        if let Some(v) = value {
            let native = ctx.native_pixels_per_point().unwrap_or(1.0).max(0.1);
            ctx.set_zoom_factor(v / native);
            ctx.request_repaint();
        }
        Ok(json!({ "ui_scale": r2(value.unwrap_or_else(|| ctx.pixels_per_point())), "saved": false, "note": "tests only: the next start takes the monitor's scale again" }))
    })
    .await
}

fn ui_contract_json() -> Value {
    let pairs = |list: &[(&str, &str)]| {
        let mut m = Map::new();
        for (k, v) in list {
            m.insert((*k).to_owned(), json!(v));
        }
        Value::Object(m)
    };
    json!({
        "tile_range": [contract::TILE_MIN, contract::TILE_MAX],
        "tile_default": contract::TILE_DEFAULT,
        "tile_default_panel": contract::TILE_DEFAULT_PANEL,
        "view_default": "tiles",
        "view_default_panel": "list",
        "tile_error": contract::TILE_RANGE_ERROR,
        "theme_keys": contract::THEME_PRESETS.iter().map(|(k, _)| *k).collect::<Vec<_>>(),
        "categories": contract::CATEGORIES,
        "card_menu": [contract::CARD_SHOW_IN_EXPLORER, contract::CARD_DELETE, contract::CARD_REMOVE_FROM_LIST],
        "delete_project_dialog": contract::DELETE_PROJECT_DIALOG,
        "panels": pairs(&contract::PANELS),
        "drawers": pairs(&contract::DRAWERS),
        "dialogs": pairs(&contract::DIALOGS),
        "document_kinds": contract::DOCUMENT_KINDS,
        "popups": contract::POPUPS,
        "menus": contract::MENUS,
        "viewport_menus": contract::VIEWPORT_MENUS,
        "kit_dialogs": contract::KIT_DIALOGS,
        "palette": contract::PALETTE_ID,
        "ui_scale_range": [contract::UI_SCALE_MIN, contract::UI_SCALE_MAX],
    })
}

// ---- window_control, titlebar_info, reset_layout ----------------------------------------------------

/// save= or discard= decides what happens to unsaved tabs; neither refuses with the list
/// ("unsaved"), except under -nodlg, where they are discarded as the exit prompt always answered
/// there. Returns the fields that say what was done.
fn settle_unsaved(
    shell: &mut ShellState,
    ctx: &Context,
    save: bool,
    mut discard: bool,
    unattended: bool,
) -> Result<Map<String, Value>, CmdError> {
    if save && discard {
        return Err(invalid("save and discard exclude each other"));
    }
    let dirty = shell.dirty_tabs();
    let mut done = Map::new();
    if dirty.is_empty() {
        return Ok(done);
    }
    if !save && !discard {
        if !unattended {
            return Err(unsaved(&dirty));
        }
        discard = true;
    }
    for (id, label) in &dirty {
        if discard {
            shell.discard_tab(ctx, id);
        } else if let Err(e) = shell.save_tab(ctx, id) {
            return Err(invalid(format!("{label}: {e}")));
        }
    }
    done.insert(if discard { "discarded" } else { "saved" }.into(), dirty_json(&dirty));
    Ok(done)
}

async fn window_control(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let action = str_arg(&args, "action").to_owned();
    let Ok(queue) = UiQueue::of(&editor) else {
        // without a window there is one thing left of it: closing the editor
        return match action.as_str() {
            "close" => {
                editor.refuse_unsaved(args.flag("discard"))?;
                quit_soon(&editor);
                Ok(json!({ "closing": true }))
            }
            "" => Err(invalid("missing 'action' argument")),
            other => Err(invalid(format!("'{other}' needs a window; this editor runs without one"))),
        };
    };
    let save = args.flag("save");
    let discard = args.flag("discard");
    let size = (args.f32("width"), args.f32("height"));
    let unattended = editor.flags.unattended;
    // saving on request: every area's documents and the tabs' own work are written first, and
    // the answer names the tabs that were dirty
    let mut saved_before = None;
    if action == "close" && save && !discard {
        let dirty = queue.run(|shell, _| Ok(shell.dirty_tabs())).await?;
        editor.save_all().await.map_err(|e| invalid(e.message))?;
        if !dirty.is_empty() {
            saved_before = Some(dirty_json(&dirty));
        }
    }
    let done = queue
        .run(move |shell, ctx| {
            let maximized = ctx.input(|i| i.viewport().maximized == Some(true));
            let minimized = ctx.input(|i| i.viewport().minimized == Some(true));
            match action.as_str() {
                "minimize" => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                "maximize" => ctx.send_viewport_cmd(ViewportCommand::Maximized(true)),
                "restore" => {
                    if minimized {
                        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    } else {
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
                    }
                }
                "resize" => {
                    let (Some(w), Some(h)) = size else {
                        return Err(invalid("resize takes width and height in pt, at least 320 x 240"));
                    };
                    if !(320.0..=16384.0).contains(&w) || !(240.0..=16384.0).contains(&h) {
                        return Err(invalid("resize takes width and height in pt, at least 320 x 240"));
                    }
                    // a minimized window that was maximized restores to maximized: both go first
                    if minimized {
                        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    }
                    if maximized || minimized {
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
                    }
                    ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(w, h)));
                }
                "close" => {
                    let done = settle_unsaved(shell, ctx, save, discard, unattended)?;
                    return Ok((true, done));
                }
                _ => return Err(invalid("action must be minimize, maximize, restore, resize or close")),
            }
            ctx.request_repaint();
            Ok((false, Map::new()))
        })
        .await?;
    let (closing, mut settled) = done;
    if let Some(saved) = saved_before {
        settled.insert("saved".into(), saved);
    }
    if closing {
        quit_soon(&editor);
    } else {
        // the window takes the change at the end of the frame and reports its size on the next
        editor.frames.after(2).await?;
    }
    let mut out = queue
        .run(|_, ctx| {
            let v = ctx.input(|i| i.viewport().clone());
            let ppp = ctx.pixels_per_point();
            let inner = v.inner_rect.unwrap_or(Rect::ZERO);
            let mut m = Map::new();
            m.insert("maximized".into(), json!(v.maximized == Some(true)));
            m.insert("minimized".into(), json!(v.minimized == Some(true)));
            m.insert("client".into(), json!([r0(inner.width() * ppp), r0(inner.height() * ppp)]));
            m.insert("client_pt".into(), json!([r0(inner.width()), r0(inner.height())]));
            m.insert("dpi".into(), json!(r3(ppp)));
            Ok(m)
        })
        .await
        .unwrap_or_default();
    out.extend(settled);
    Ok(Value::Object(out))
}

/// Ends the editor a moment after the answer left: the reply must reach the caller first.
fn quit_soon(editor: &Arc<Editor>) {
    let ed = editor.clone();
    editor.exec.spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        ed.request_quit();
    });
}

async fn titlebar_info(editor: Arc<Editor>, _args: Args, _origin: Origin) -> Reply {
    editor.frames.after(2).await?;
    on_ui(&editor, |shell, ctx| {
        let ppp = ctx.pixels_per_point();
        let c = shell.caption_geom();
        let px = |v: f32| (v * ppp) as i64;
        let tabs: Vec<Value> = shell
            .tab_rects()
            .into_iter()
            .map(|(id, r)| json!({ "id": id, "rect": [px(r.min.x), px(r.min.y), px(r.max.x), px(r.max.y)] }))
            .collect();
        let maximized = ctx.input(|i| i.viewport().maximized == Some(true));
        Ok(json!({
            "enabled": true,
            "band": [px(c.bar.min.x), px(c.bar.min.y), px(c.bar.max.x), px(c.bar.max.y)],
            "menus_end": px(c.menus_end),
            "buttons_x": px(c.buttons_x),
            "maximized": maximized,
            "icon": shell.has_logo(),
            "caption_h": px(c.bar.height()),
            "dpi": r2(ppp),
            "folded": shell.folded(),
            "interactive": [[px(c.menus_x0), px(c.menus_end)], [px(c.buttons_x), px(c.bar.max.x)]],
            "tabs": tabs,
        }))
    })
    .await
}

async fn reset_layout(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let scope = str_arg(&args, "scope").to_owned();
    let all = scope == "all";
    if !scope.is_empty() && !all && scope != "level" {
        return Err(invalid("scope must be level or all"));
    }
    on_ui(&editor, move |shell, ctx| {
        shell.reset_layout(ctx, all);
        Ok(())
    })
    .await?;
    Ok(if all {
        json!({ "scope": "all", "note": "the Level layout is rebuilt when Level draws next, and every split and grid width is back to its default" })
    } else {
        json!({ "scope": "level", "note": "the Level layout is rebuilt for the window's size class when Level draws next" })
    })
}

// ---- theme, preferences, shortcuts ----------------------------------------------------------------

fn theme_json(accent: gz_ui::Accent) -> Value {
    json!({
        "preset": settings::theme_key(accent),
        "name": settings::theme_name(accent),
        "default": accent == gz_ui::Accent::default(),
        "presets": contract::THEME_PRESETS.iter().map(|(k, n)| json!({ "key": k, "name": n })).collect::<Vec<_>>(),
    })
}

async fn theme(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let requested = str_arg(&args, "preset").to_owned();
    let action = args.text("action").map_or_else(
        || if requested.is_empty() { "get".to_owned() } else { "set".to_owned() },
        |a| a.to_ascii_lowercase(),
    );
    let settings = EditorSettings::of(&editor);
    let accent = match action.as_str() {
        "set" => match settings::parse_theme(&requested) {
            Some(a) => Some(a),
            None => {
                return Err(CmdError::invalid("unknown theme preset")
                    .with_data(json!({ "accepted": ["grizzly-purple", "graphite"] })));
            }
        },
        "reset" => Some(gz_ui::Accent::default()),
        "get" => None,
        _ => return Err(invalid("action must be get, set, or reset")),
    };
    if let Some(a) = accent {
        settings.set_accent(a);
        settings.save_soon(&editor);
        if let Ok(q) = UiQueue::of(&editor) {
            q.run(move |_, ctx| {
                gz_ui::theme::install(ctx, a);
                ctx.request_repaint();
                Ok(())
            })
            .await?;
        }
    }
    Ok(theme_json(settings.accent()))
}

async fn preferences(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let settings = EditorSettings::of(&editor);
    let action = args.str_or("action", "get").to_owned();
    let key = str_arg(&args, "key").to_owned();
    if !matches!(action.as_str(), "get" | "set" | "show") {
        return Err(invalid("action must be get, set or show"));
    }
    // the Preferences tab on a page (a top-level folder of its rows, or Keyboard), with its search
    if action == "show" {
        let page = str_arg(&args, "page").to_owned();
        let search = str_arg(&args, "search").to_owned();
        if !page.is_empty() {
            *settings.page.lock() = gz_editor::SmolStr::new(&page);
        }
        let shown = settings.page.lock().to_string();
        let arg = if search.is_empty() { shown.clone() } else { format!("{shown}|{search}") };
        let active = on_ui(&editor, move |shell, ctx| {
            shell
                .open(
                    ctx,
                    OpenRequest { arg: Some(arg), ..OpenRequest::kind("preferences") },
                    false,
                    Asker::Mcp,
                )
                .map_err(|e| match e {
                    OpenError::Unsaved { id, label } => unsaved(&[(id, label)]),
                    OpenError::Other(e) => invalid(e),
                })?;
            Ok(shell.active().to_owned())
        })
        .await?;
        return Ok(json!({ "page": shown, "active_tab": active }));
    }
    let def =
        if key.is_empty() {
            None
        } else {
            Some(wire_pref_def(&key).ok_or_else(|| {
                invalid(format!("no preference '{key}' (preferences action=get lists them)"))
            })?)
        };
    if action == "set" {
        let (Some(def), Some(value)) = (def, text_arg(&args, "value")) else {
            return Err(invalid("set takes key= and value="));
        };
        let v = parse_pref(def, &value).map_err(|e| invalid(format!("{} takes {e}", def.key)))?;
        settings.set_pref(def.key, v);
        settings.save_soon(&editor);
    }
    if let Some(def) = def {
        let mut out = Map::new();
        out.insert("key".into(), json!(def.key));
        out.insert("value".into(), pref_json(def, settings.pref(def.key).unwrap_or(def.default)));
        if def.ty == PrefType::Angle {
            out.insert("unit".into(), json!("deg"));
        }
        if def.key == "floating_panels" {
            out.insert("note".into(), json!("takes effect after a restart"));
        }
        return Ok(Value::Object(out));
    }
    let mut all = Map::new();
    for d in PREFS.iter().take(WIRE_PREFS) {
        all.insert(d.key.to_owned(), pref_json(d, settings.pref(d.key).unwrap_or(d.default)));
    }
    let page = settings.page.lock().to_string();
    Ok(json!({ "preferences": all, "page": page, "angles": "deg", "theme": "see the theme command" }))
}

async fn shortcuts(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let action = args.str_or("action", "list").to_owned();
    let command = str_arg(&args, "command").to_owned();
    let keys = str_arg(&args, "keys").to_owned();
    let filter = str_arg(&args, "filter").to_owned();
    on_ui(&editor, move |shell, _ctx| {
        if action == "list" {
            let rows: Vec<Value> = shell
                .shortcut_rows()
                .into_iter()
                .filter(|r| gz_ui::search::matches(&filter, &format!("{} {} {}", r.0, r.1, r.2)))
                .map(|(id, label, keys)| json!({ "id": id, "label": label, "keys": keys }))
                .collect();
            return Ok(json!({ "shortcuts": rows }));
        }
        if action != "set" && action != "clear" {
            return Err(invalid("action must be list, set or clear"));
        }
        let target = shell.shortcut_target(&command).ok_or_else(|| {
            invalid(format!("no command '{command}' takes a shortcut (shortcuts action=list names them)"))
        })?;
        let chord = if action == "set" {
            Some(
                Chord::parse(&keys)
                    .ok_or_else(|| invalid("keys must be a shortcut such as Ctrl+Shift+S, F5 or Num Del"))?,
            )
        } else {
            None
        };
        let moved_from = shell.bind(target, chord);
        let row = shell
            .shortcut_row(target)
            .map_or(Value::Null, |(id, label, keys)| json!({ "id": id, "label": label, "keys": keys }));
        Ok(json!({ "shortcut": row, "moved_from": moved_from.map_or(Value::Null, Value::String) }))
    })
    .await
}

// ---- command_palette, progress ------------------------------------------------------------------

async fn command_palette(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let query = str_arg(&args, "query").to_owned();
    let id = str_arg(&args, "id").to_owned();
    let action = args.text("action").map_or_else(
        || if query.is_empty() { "state".to_owned() } else { "query".to_owned() },
        str::to_owned,
    );
    let command = args.i64("command");
    let limit = args.i64("limit").unwrap_or(50).clamp(1, 250) as usize;
    on_ui(&editor, move |shell, ctx| {
        match action.as_str() {
            "open" => shell.open_palette(&query),
            "close" => shell.close_palette(),
            "execute" => {
                let executed = if !id.is_empty() {
                    shell.palette_execute(ctx, &id)
                } else {
                    match command.and_then(|c| usize::try_from(c).ok()) {
                        Some(c) => shell.run_command(ctx, c),
                        None => false,
                    }
                };
                if !executed {
                    return Err(invalid("unknown or unavailable command; query the palette first"));
                }
                shell.close_palette();
            }
            "query" | "state" => {}
            _ => return Err(invalid("action must be state, query, open, close, or execute")),
        }
        let rows = pal::rank(shell.palette_rows(), &query, limit);
        Ok(json!({ "open": shell.palette_open(), "query": query, "results": rows.iter().map(PaletteRow::to_json).collect::<Vec<_>>() }))
    })
    .await
}

use crate::palette::PaletteRow;

async fn progress(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let action = args.str_or("action", "get").to_owned();
    let running: Vec<gz_editor::JobInfo> =
        editor.jobs.snapshot().into_iter().filter(gz_editor::JobInfo::running).collect();
    match action.as_str() {
        "cancel" => {
            if running.is_empty() {
                return Err(invalid("no active operation"));
            }
            for job in &running {
                editor.jobs.cancel(job.id);
            }
        }
        "get" => {}
        _ => return Err(invalid("action must be get or cancel")),
    }
    let ids: Vec<gz_editor::JobId> = running.iter().map(|j| j.id).collect();
    let cancel_mark = action == "cancel";
    let (cancelled, log) = match UiQueue::of(&editor) {
        Ok(q) => q
            .run(move |shell, _| {
                if cancel_mark {
                    for id in &ids {
                        shell.cancelled.insert(*id);
                    }
                }
                Ok((shell.cancelled.iter().copied().collect::<Vec<_>>(), shell.log_state()))
            })
            .await
            .unwrap_or_else(|_| (Vec::new(), default_log_state(&editor))),
        Err(_) => (
            if cancel_mark { running.iter().map(|j| j.id).collect() } else { Vec::new() },
            default_log_state(&editor),
        ),
    };
    let now = crate::tasks::now_ms();
    let background: Vec<Value> = running
        .iter()
        .map(|j| {
            crate::tasks::task_json(
                j,
                crate::tasks::card_of(&editor, j.id).as_ref(),
                cancelled.contains(&j.id),
                now,
            )
        })
        .collect();
    Ok(json!({
        "active": false,
        "cancelable": false,
        "cancel_requested": false,
        "tasks": [],
        "background": background,
        "errors": log.errors,
        "warnings": log.warnings,
        "status_line": log.last,
    }))
}

// ---- list_commands, exec_command -------------------------------------------------------------

/// The keys a face of the table is bound to now, as text.
fn face_keys(shell: &ShellState, face: Option<usize>) -> String {
    face.and_then(|i| shell.binding(i)).map(|c| c.label()).unwrap_or_default()
}

async fn list_commands(editor: Arc<Editor>, _args: Args, _origin: Origin) -> Reply {
    on_ui(&editor, |shell, _| {
        let rows: Vec<Value> = shell
            .parts
            .cpp
            .rows
            .iter()
            .map(|r| {
                let mut row = Map::new();
                row.insert("id".into(), json!(r.cmd.id));
                row.insert("name".into(), json!(r.cmd.name));
                if !r.cmd.desc.is_empty() {
                    row.insert("menu".into(), json!(r.cmd.desc));
                }
                // presets beyond the implicit one are the documented parameter combinations
                if r.cmd.subs.len() > 1 {
                    let presets: Vec<Value> = r
                        .cmd
                        .subs
                        .iter()
                        .zip(&r.faces)
                        .map(|(s, f)| crate::cpp::preset_json(s, &face_keys(shell, *f)))
                        .collect();
                    row.insert("presets".into(), Value::Array(presets));
                }
                row.insert("keys".into(), json!(face_keys(shell, r.faces.first().copied().flatten())));
                Value::Object(row)
            })
            .collect();
        Ok(json!({ "commands": rows }))
    })
    .await
}

/// What a command run came to: the row, the answer, and why it refused when it did.
pub(crate) struct Ran {
    pub id: u32,
    pub name: String,
    pub result: CommandVar,
    pub refused: Option<String>,
}

/// Why a command did not run at all.
pub(crate) enum NotRun {
    Unknown,
    NoSub(String),
    Other(CmdError),
}

/// What the UI thread found and did.
enum Step {
    Done(Ran),
    Call(u32, String, &'static str, Args),
    Task(u32, String, crate::cpp::TaskFn, CommandVar, CommandVar),
}

/// Runs a row of the C++ table - the one `exec_command`, a macro's line and a face run - with
/// its sub-command (named, or picked by the parameters) and the parameters given over the
/// sub-command's own. Off the UI thread; what the row does there runs there.
pub(crate) async fn exec(
    editor: &Arc<Editor>,
    id: Option<i64>,
    name: String,
    sub: Option<String>,
    p1: Option<CommandVar>,
    p2: Option<CommandVar>,
    depth: usize,
) -> Result<Ran, NotRun> {
    let step = on_ui(editor, move |shell, ctx| {
        let Some(at) = shell.parts.cpp.find(id, &name) else { return Ok(Err(NotRun::Unknown)) };
        let Some(row) = shell.parts.cpp.rows.get(at) else { return Ok(Err(NotRun::Unknown)) };
        // a sub-command is named by a line (`NAME.Sub(..)`) or by the name (`NAME."Sub"`)
        let named = sub.filter(|s| !s.is_empty()).or_else(|| {
            name.split_once('.')
                .map(|(_, s)| s.trim_matches('"').to_owned())
                .filter(|s| !s.is_empty() && !row.cmd.desc.eq_ignore_ascii_case(&name))
        });
        let k = match &named {
            Some(s) => match row.cmd.subs.iter().position(|x| x.desc.eq_ignore_ascii_case(s)) {
                Some(k) => k,
                None => return Ok(Err(NotRun::NoSub(s.clone()))),
            },
            None => crate::cpp::pick_sub(&row.cmd.subs, p1.as_ref(), p2.as_ref()).unwrap_or(0),
        };
        let preset = row.cmd.subs.get(k).cloned();
        let a = p1.or_else(|| preset.as_ref().map(|s| s.p1.clone())).unwrap_or_default();
        let b = p2.or_else(|| preset.as_ref().map(|s| s.p2.clone())).unwrap_or_default();
        let (row_id, row_name) = (row.cmd.id, row.cmd.name.to_string());
        let action = row.cmd.action.clone();
        let face = row.faces.get(k).copied().flatten();
        let enabled = row.cmd.enabled.clone();
        crate::cpp::log_command(&shell.editor, &row_name, &a, &b, depth);
        let done = |result: CommandVar, refused: Option<String>| {
            Ok(Ok(Step::Done(Ran { id: row_id, name: row_name.clone(), result, refused })))
        };
        if let Some(f) = enabled
            && !f(&shell.editor, &shell.active)
        {
            return done(CommandVar::Int(0), Some("the command is not available now".to_owned()));
        }
        if matches!(action, CppAction::Ui) {
            // logged above, with its depth
            shell.quiet_log = true;
            let ran = face.is_some_and(|i| shell.run_command(ctx, i));
            shell.quiet_log = false;
            let why = (!ran).then(|| "the command is not available now".to_owned());
            return done(CommandVar::Int(u32::from(ran)), why);
        }
        let outcome = crate::cpp::run_now(&mut crate::shell::cx!(shell, ctx), &action, &a, &b);
        match outcome {
            crate::cpp::Outcome::Done(v) => done(v, None),
            crate::cpp::Outcome::Refused(why) => done(CommandVar::Int(0), Some(why)),
            crate::cpp::Outcome::Call(c, args) => Ok(Ok(Step::Call(row_id, row_name, c, args))),
            crate::cpp::Outcome::Task(t, a, b) => Ok(Ok(Step::Task(row_id, row_name, t, a, b))),
        }
    })
    .await
    .map_err(NotRun::Other)??;
    Ok(match step {
        Step::Done(r) => r,
        Step::Call(id, name, command, args) => {
            let reply = editor.call(command, args, editor.ui_origin()).await;
            if reply.get("ok").and_then(Value::as_bool) == Some(true) {
                Ran { id, name, result: CommandVar::Int(1), refused: None }
            } else {
                let why = reply.get("error").and_then(Value::as_str).unwrap_or("the command failed").to_owned();
                Ran { id, name, result: CommandVar::Int(0), refused: Some(why) }
            }
        }
        Step::Task(id, name, task, a, b) => match task(editor.clone(), a, b).await {
            Ok(v) => Ran { id, name, result: v, refused: None },
            Err(why) => Ran { id, name, result: CommandVar::Int(0), refused: Some(why) },
        },
    })
}

/// One line of a command list (`ExecuteCommandList`): the first that is unknown or does not
/// run is the error, in the C++ words.
pub(crate) async fn exec_line(editor: &Arc<Editor>, line: &crate::macros::Line, depth: usize) -> Result<(), String> {
    let sub = (!line.sub.is_empty()).then(|| line.sub.clone());
    match exec(editor, None, line.name.clone(), sub, line.p1.clone(), line.p2.clone(), depth).await {
        Ok(r) if r.refused.is_none() && r.result != CommandVar::Int(0) => Ok(()),
        Ok(r) => {
            let why = r.refused.map(|w| format!(" - {w}")).unwrap_or_default();
            Err(format!("Can't execute command: '{}'{why}", line.text))
        }
        Err(NotRun::Unknown) => Err(format!("Can't find command: '{}'", line.text)),
        Err(NotRun::NoSub(s)) => Err(format!("Can't find sub-command: '{s}'")),
        Err(NotRun::Other(e)) => Err(e.message),
    }
}

/// A parameter as `exec_command` takes it: `p1s` a text, `p1i` a number (`-1` is none).
fn param(args: &Args, s: &str, i: &str) -> Option<CommandVar> {
    if let Some(t) = args.text(s) {
        return Some(CommandVar::Str(t.to_owned()));
    }
    args.i64(i).filter(|v| *v >= 0).map(|v| CommandVar::Int(u32::try_from(v).unwrap_or(u32::MAX)))
}

async fn exec_command(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    // an escaped "Edit\Undo" may arrive with both slashes
    let name = str_arg(&args, "name").replace("\\\\", "\\");
    let id = args.i64("id").filter(|v| *v >= 0);
    let (p1, p2) = (param(&args, "p1s", "p1i"), param(&args, "p2s", "p2i"));
    if id.is_none() && name.is_empty() {
        return Err(invalid("unknown command - see list_commands"));
    }
    match exec(&editor, id, name, None, p1, p2, 0).await {
        Ok(r) => {
            let mut out = Map::new();
            out.insert("id".into(), json!(r.id));
            out.insert("name".into(), json!(r.name));
            out.insert("result".into(), r.result.json());
            if let Some(why) = r.refused {
                out.insert("refused".into(), json!(why));
            }
            Ok(Value::Object(out))
        }
        Err(NotRun::Unknown) => Err(invalid("unknown command - see list_commands")),
        Err(NotRun::NoSub(s)) => Err(invalid(format!("Can't find sub-command: '{s}'"))),
        Err(NotRun::Other(e)) => Err(e),
    }
}

/// Registers the commands of the frame. Call it after the commands it replaces (the headless
/// `window_control`): a name registered twice keeps the later handler.
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "state", state);
    reg.add(AREA, "ui_list", ui_list);
    reg.add(AREA, "ui_tab", ui_tab);
    reg.add(AREA, "ui_panel", ui_panel);
    reg.add(AREA, "ui_drawer", ui_drawer);
    reg.add(AREA, "ui_dialog", ui_dialog);
    reg.add(AREA, "ui_popup", ui_popup);
    reg.add(AREA, "ui_toast", ui_toast);
    reg.add(AREA, "ui_scroll", ui_scroll);
    reg.add(AREA, "ui_split", ui_split);
    reg.add(AREA, "ui_audit", ui_audit);
    reg.add(AREA, "ui_scale", ui_scale);
    reg.add_sync(AREA, "ui_contract", |_, _, _| Ok(ui_contract_json()));
    reg.add(AREA, "window_control", window_control);
    reg.add(AREA, "titlebar_info", titlebar_info);
    reg.add(AREA, "reset_layout", reset_layout);
    reg.add(AREA, "theme", theme);
    reg.add(AREA, "preferences", preferences);
    reg.add(AREA, "shortcuts", shortcuts);
    reg.add(AREA, "command_palette", command_palette);
    reg.add(AREA, "progress", progress);
    reg.add(AREA, "list_commands", list_commands);
    reg.add(AREA, "exec_command", exec_command);
    crate::shots::register(reg);
}

#[cfg(test)]
mod tests {
    use super::{dialog_mcp_id, ui_contract_json};

    #[test]
    fn the_contract_names_what_qa_pins() {
        let c = ui_contract_json();
        assert_eq!(c["palette"], "grizzly_command_palette");
        assert!(c["menus"].as_array().is_some_and(|m| m.iter().any(|v| v == "Menu")));
        assert_eq!(c["drawers"].as_object().map(|d| d.len()), Some(2));
        assert_eq!(c["panels"].as_object().map(|d| d.len()), Some(7));
    }

    #[test]
    fn kit_dialogs_answer_to_the_contract_ids() {
        assert_eq!(dialog_mcp_id("dlg.import_base_scene"), "import_base_scene");
        assert_eq!(dialog_mcp_id("dlg.catalog_submit"), "submit");
        assert_eq!(dialog_mcp_id("dlg.delete_project"), "dlg.delete_project");
    }
}
