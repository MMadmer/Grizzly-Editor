//! `state`, `app.*`, `doc.*` and `perf.stalls`: the program, its documents and its stalls
//! (docs/COMMANDS.md, section 5).

use std::sync::Arc;
use std::time::Duration;

use egui::{Rect, ViewportCommand};
use gz_editor::{CmdError, Editor, NoParams, Origin, Registry, Reply, Spec, Value, json};
use gz_ui::Tone;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Map;

use std::future::Future;

use crate::actions::parse_keys;
use crate::cmds::{asker_of, dirty_json, invalid, pt, scale, top_modal, unsaved};
use crate::doc::{EditAction, OpenRequest};
use crate::queue::{UiQueue, on_ui};
use crate::settings::{self, EditorSettings, PREFS, PrefType, parse_pref, pref_json};
use crate::shell::{OpenError, ShellState};

// ---- state ------------------------------------------------------------------------------------

async fn state(editor: Arc<Editor>, _p: NoParams, _origin: Origin) -> Reply {
    let info = crate::project::project_info(&editor);
    let mut out = Map::new();
    out.insert(
        "project".into(),
        json!(
            info.active.then(
                || json!({ "folder": info.folder, "name": info.name, "game_linked": info.game_linked })
            )
        ),
    );
    let jobs: Vec<Value> = editor
        .jobs
        .snapshot()
        .into_iter()
        .filter(gz_editor::JobInfo::running)
        .map(|j| json!({ "job": j.id.0, "kind": j.kind, "title": j.title, "stage": j.stage, "fraction": j.fraction, "owner": j.owner }))
        .collect();
    out.insert("jobs".into(), Value::Array(jobs));
    let log = crate::shell::JobMarks::of(&editor).log_state(&editor);
    out.insert("problems".into(), json!({ "errors": log.errors, "warnings": log.warnings }));
    out.insert("unsaved".into(), serde_json::to_value(editor.dirty_docs()).unwrap_or(Value::Null));
    let ui = match UiQueue::of(&editor) {
        Ok(q) => Some(
            q.run(|shell, ctx| {
                let level = shell.tab_json(crate::tabs::LEVEL);
                let scene = level.as_ref().and_then(|t| t.get("path")).and_then(Value::as_str).unwrap_or("").replace('\\', "/");
                let tabs: Vec<Value> = shell
                    .tabs_json()
                    .into_iter()
                    .map(|t| json!({ "id": t["id"], "kind": t["kind"], "label": t["label"], "path": t["path"], "dirty": t["dirty"], "active": t["active"], "missing": t["missing"] }))
                    .collect();
                let modal = top_modal(shell, ctx).map_or(Value::Null, |(title, id, _)| json!({ "title": title, "id": id }));
                Ok(json!({ "scene": scene, "tabs": tabs, "active_tab": shell.active(), "modal": modal, "start_page": shell.start_page(), "scale": scale(ctx.pixels_per_point()) }))
            })
            .await?,
        ),
        Err(_) => None,
    };
    out.insert("window".into(), json!(ui.is_some()));
    match ui {
        Some(Value::Object(m)) => out.extend(m),
        _ => {
            out.insert("scene".into(), json!(""));
            out.insert("tabs".into(), json!([]));
            out.insert("active_tab".into(), json!(""));
            out.insert("modal".into(), Value::Null);
            out.insert("start_page".into(), json!(!(info.active && info.game_linked)));
        }
    }
    Ok(Value::Object(out))
}

// ---- app.window, app.quit ------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum WindowAction {
    /// Where the window is and how big, and whether it is minimised or maximised.
    #[default]
    State,
    Minimize,
    Restore,
    Maximize,
    /// To `width` x `height` pt (at least 320 x 240).
    Resize,
    /// Its outer top-left corner to `x`, `y` pt.
    Move,
    /// Brings it to the front.
    Focus,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WindowParams {
    /// What to do.
    #[serde(default)]
    action: WindowAction,
    /// The width for resize, in pt.
    #[serde(default)]
    width: Option<f32>,
    /// The height for resize, in pt.
    #[serde(default)]
    height: Option<f32>,
    /// The left edge for move, in pt.
    #[serde(default)]
    x: Option<f32>,
    /// The top edge for move, in pt.
    #[serde(default)]
    y: Option<f32>,
}

async fn app_window(editor: Arc<Editor>, p: WindowParams, _origin: Origin) -> Reply {
    let queue = UiQueue::of(&editor)?;
    let size = (p.width, p.height);
    if p.action == WindowAction::Resize {
        let (Some(w), Some(h)) = size else {
            return Err(invalid("resize takes width and height in pt, at least 320 x 240"));
        };
        if !(320.0..=16384.0).contains(&w) || !(240.0..=16384.0).contains(&h) {
            return Err(invalid("resize takes width and height in pt, at least 320 x 240"));
        }
    }
    if p.action == WindowAction::Move && (p.x.is_none() || p.y.is_none()) {
        return Err(invalid("move takes x and y in pt"));
    }
    let action = p.action;
    let (x, y) = (p.x, p.y);
    queue
        .run(move |_, ctx| {
            let maximized = ctx.input(|i| i.viewport().maximized == Some(true));
            let minimized = ctx.input(|i| i.viewport().minimized == Some(true));
            match action {
                WindowAction::State => {}
                WindowAction::Minimize => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                WindowAction::Maximize => ctx.send_viewport_cmd(ViewportCommand::Maximized(true)),
                WindowAction::Restore => {
                    if minimized {
                        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    } else {
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
                    }
                }
                WindowAction::Resize => {
                    // a minimised window that was maximised restores to maximised: both go first
                    if minimized {
                        ctx.send_viewport_cmd(ViewportCommand::Minimized(false));
                    }
                    if maximized || minimized {
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(false));
                    }
                    if let (Some(w), Some(h)) = size {
                        ctx.send_viewport_cmd(ViewportCommand::InnerSize(egui::vec2(w, h)));
                    }
                }
                WindowAction::Move => {
                    if let (Some(x), Some(y)) = (x, y) {
                        ctx.send_viewport_cmd(ViewportCommand::OuterPosition(egui::pos2(x, y)));
                    }
                }
                WindowAction::Focus => ctx.send_viewport_cmd(ViewportCommand::Focus),
            }
            ctx.request_repaint();
            Ok(())
        })
        .await?;
    // the window takes the change at the end of the frame and reports its size on the next
    editor.frames.after(2).await?;
    if let (WindowAction::Resize, (Some(w), Some(h))) = (action, size) {
        // answered once frames are drawn at the size asked for: what the caller asks next is
        // worked out for it; a size the screen cannot hold is clamped, and one that holds still
        // is the one the window got
        let mut last = egui::Vec2::ZERO;
        let mut still = 0;
        for _ in 0..100 {
            let got = queue.run(|_, ctx| Ok(ctx.content_rect().size())).await.unwrap_or_default();
            still = if (got - last).length() < 0.5 { still + 1 } else { 0 };
            last = got;
            if ((got.x - w).abs() <= 1.0 && (got.y - h).abs() <= 1.0) || still >= 10 {
                editor.frames.after(1).await?;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
            editor.exec.wake().ping();
        }
    }
    queue
        .run(|_, ctx| {
            let v = ctx.input(|i| i.viewport().clone());
            let inner = v.inner_rect.unwrap_or(Rect::ZERO);
            let outer = v.outer_rect.unwrap_or(Rect::ZERO);
            Ok(json!({
                "maximized": v.maximized == Some(true),
                "minimized": v.minimized == Some(true),
                "focused": v.focused == Some(true),
                "size": [pt(inner.width()), pt(inner.height())],
                "position": [pt(outer.min.x), pt(outer.min.y)],
                "scale": scale(ctx.pixels_per_point()),
            }))
        })
        .await
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct QuitParams {
    /// Save every unsaved document first.
    #[serde(default)]
    save: bool,
    /// Drop every unsaved change.
    #[serde(default)]
    discard: bool,
}

/// Ends the editor a moment after the answer left: the reply must reach the caller first.
fn quit_soon(editor: &Arc<Editor>) {
    let ed = editor.clone();
    editor.exec.spawn(async move {
        tokio::time::sleep(Duration::from_millis(150)).await;
        ed.request_quit();
    });
}

async fn app_quit(editor: Arc<Editor>, p: QuitParams, origin: Origin) -> Reply {
    if p.save && p.discard {
        return Err(invalid("save and discard exclude each other"));
    }
    // a person: the question about unsaved work, then the editor ends by itself
    if origin == Origin::Ui
        && !p.save
        && !p.discard
        && let Ok(q) = UiQueue::of(&editor)
    {
        q.run(|shell, ctx| {
            shell.request_exit(ctx);
            Ok(())
        })
        .await?;
        return Ok(json!({ "asking": true }));
    }
    // the window's own action under -nodlg has the logged answer; a command's caller says discard
    let unattended = origin == Origin::Unattended;
    let mut out = Map::new();
    if p.save {
        let dirty = match UiQueue::of(&editor) {
            Ok(q) => q.run(|shell, _| Ok(shell.dirty_tabs())).await?,
            Err(_) => Vec::new(),
        };
        editor.save_all().await.map_err(|e| CmdError::new("failed", e.message))?;
        // what a tab still holds is saved and waited for: the editor does not end over a write under way
        let left: Vec<String> = match UiQueue::of(&editor) {
            Ok(q) => q.run(|shell, _| Ok(shell.dirty_tabs().into_iter().map(|(id, _)| id).collect())).await?,
            Err(_) => Vec::new(),
        };
        crate::unsaved::save_tabs(&editor, left, origin.may_ask()).await?;
        if !dirty.is_empty() {
            out.insert("saved".into(), dirty_json(&dirty));
        }
    } else {
        // the window's tabs and every area's documents: refused with what is unsaved unless the
        // caller drops it (or runs unattended, where the exit question always answered so)
        let dirty = match UiQueue::of(&editor) {
            Ok(q) => q.run(|shell, _| Ok(shell.dirty_tabs())).await?,
            Err(_) => Vec::new(),
        };
        if !p.discard && !unattended {
            if !dirty.is_empty() {
                return Err(unsaved(&dirty));
            }
            editor.refuse_unsaved(false)?;
        }
        if let Ok(q) = UiQueue::of(&editor) {
            q.run(|shell, ctx| {
                for (id, _) in shell.dirty_tabs() {
                    shell.discard_tab(ctx, &id);
                }
                Ok(())
            })
            .await?;
        }
        // the dropped work leaves the stores too, so the project closes clean (recovery included)
        editor.discard_all().await?;
        if !dirty.is_empty() {
            out.insert("discarded".into(), dirty_json(&dirty));
        }
    }
    quit_soon(&editor);
    out.insert("closing".into(), json!(true));
    Ok(Value::Object(out))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SwitchParams {
    /// Save the scene and every unsaved tab first; the project closes once they are written.
    #[serde(default)]
    save: bool,
    /// Drop their unsaved changes.
    #[serde(default)]
    discard: bool,
}

async fn app_switch_project(editor: Arc<Editor>, p: SwitchParams, origin: Origin) -> Reply {
    if p.save && p.discard {
        return Err(invalid("save and discard exclude each other"));
    }
    editor.project()?;
    // a person who has not answered yet: the window's question, which names the scene and every
    // other unsaved tab (with nothing unsaved the project closes at once)
    if origin == Origin::Ui && !p.save && !p.discard {
        let asking = on_ui(&editor, |shell, ctx| {
            let asking = !shell.dirty_tabs().is_empty();
            shell.switch_project(ctx);
            Ok(asking)
        })
        .await?;
        return Ok(if asking { json!({ "asking": true }) } else { json!({ "closing": true }) });
    }
    // the caller's own answer goes to the project's close, which saves every document - the scene,
    // the tabs - and waits for the saves, refuses with `unsaved` naming them, or drops them
    let mut a = gz_editor::Args::new();
    if p.save {
        a.insert("save".into(), json!(true));
    }
    if p.discard {
        a.insert("discard".into(), json!(true));
    }
    let reply = editor.call("project.close", a, origin).await;
    if reply.get("ok").and_then(Value::as_bool) != Some(true) {
        return Err(failure_of(reply, "the project did not close"));
    }
    let mut out = Map::new();
    out.insert("closed".into(), json!(true));
    for key in ["saved", "discarded"] {
        if let Some(docs) = reply.get(key) {
            out.insert(key.into(), docs.clone());
        }
    }
    Ok(Value::Object(out))
}

/// A command's answer turned back into the error it was: its code, its sentence and what else it
/// carried (the unsaved documents).
fn failure_of(reply: Value, fallback: &str) -> CmdError {
    let Value::Object(mut map) = reply else { return CmdError::new("failed", fallback) };
    let text = |v: Option<Value>| v.and_then(|v| v.as_str().map(str::to_owned));
    let code = text(map.remove("code")).unwrap_or_else(|| "failed".to_owned());
    let message = text(map.remove("error")).unwrap_or_else(|| fallback.to_owned());
    map.remove("ok");
    let error = CmdError::new(&code, message);
    if map.is_empty() { error } else { error.with_data(Value::Object(map)) }
}

async fn app_about(editor: Arc<Editor>, _p: NoParams, origin: Origin) -> Reply {
    let text = format!(
        "Grizzly Editor {} - the level and mod editor of Dead Air Refined.\nAgents drive it over MCP on 127.0.0.1.",
        env!("CARGO_PKG_VERSION")
    );
    if origin != Origin::Mcp
        && let Ok(q) = UiQueue::of(&editor)
    {
        let t = text.clone();
        q.run(move |shell, ctx| {
            if let Some(line) =
                gz_ui::dialog::notice(ctx, shell.asker(), Tone::Info, "About Grizzly Editor", &t)
            {
                shell.editor.log.finding(&line);
            }
            Ok(())
        })
        .await?;
    }
    Ok(json!({ "name": "Grizzly Editor", "version": env!("CARGO_PKG_VERSION"), "text": text }))
}

// ---- app.prefs ----------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PrefsAction {
    /// Every preference, or the one `key` names.
    #[default]
    Get,
    /// Sets `key` to `value`: it applies at once and is saved.
    Set,
    /// Opens Preferences on a page, with a search.
    Show,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PrefsParams {
    /// What to do.
    #[serde(default)]
    action: PrefsAction,
    /// A preference's key (`app.prefs` lists them): `accent`, `scale`, `view_fov`, ...
    #[serde(default)]
    key: Option<String>,
    /// Its new value: a number, true or false, a colour as 0xAARRGGBB or #RRGGBB, a name of a
    /// choice (accent: grizzly-purple or graphite; scale: auto, 100 ... 200).
    #[serde(default)]
    value: Option<String>,
    /// The page to show: General, Interface, Viewport, Tools, Objects, Recovery, Keyboard, or one
    /// an area added.
    #[serde(default)]
    page: Option<String>,
    /// A search to show the page with.
    #[serde(default)]
    search: Option<String>,
}

/// The preferences of the window itself that are no number of the table: the accent and the
/// interface scale.
fn named_pref(settings: &EditorSettings, key: &str) -> Option<Value> {
    match key {
        "accent" => Some(json!(settings::theme_key(settings.accent()))),
        "scale" => Some(json!(
            settings
                .interface_scale()
                .map_or_else(|| "auto".to_owned(), |s| format!("{}", (s * 100.0).round() as i32))
        )),
        _ => None,
    }
}

async fn app_prefs(editor: Arc<Editor>, p: PrefsParams, origin: Origin) -> Reply {
    let settings = EditorSettings::of(&editor);
    if p.action == PrefsAction::Show {
        if let Some(page) = p.page.as_deref().filter(|s| !s.is_empty()) {
            *settings.page.lock() = gz_editor::SmolStr::new(page);
        }
        let shown = settings.page.lock().to_string();
        let search = p.search.unwrap_or_default();
        let arg = if search.is_empty() { shown.clone() } else { format!("{shown}|{search}") };
        let asker = asker_of(origin);
        let active = on_ui(&editor, move |shell, ctx| {
            shell
                .open(ctx, OpenRequest { arg: Some(arg), ..OpenRequest::kind("preferences") }, false, asker)
                .map_err(|e| match e {
                    OpenError::Unsaved { id, label } => unsaved(&[(id, label)]),
                    OpenError::Other(e) => invalid(e),
                })?;
            Ok(shell.active().to_owned())
        })
        .await?;
        return Ok(json!({ "page": shown, "active_tab": active }));
    }
    let key = p.key.unwrap_or_default();
    if p.action == PrefsAction::Set {
        let value = p.value.ok_or_else(|| invalid("set takes key= and value="))?;
        match key.as_str() {
            "accent" => {
                let accent = settings::parse_theme(&value)
                    .ok_or_else(|| invalid("accent must be grizzly-purple or graphite"))?;
                settings.set_accent(accent);
                if let Ok(q) = UiQueue::of(&editor) {
                    q.run(move |_, ctx| {
                        gz_ui::theme::install(ctx, accent);
                        ctx.request_repaint();
                        Ok(())
                    })
                    .await?;
                }
            }
            "scale" => {
                let s = settings::parse_interface_scale(&value)
                    .ok_or_else(|| invalid("scale must be auto, 100, 125, 150, 175 or 200"))?;
                settings.set_interface_scale(s);
                editor.exec.wake().ping();
            }
            _ => {
                let def = settings.def(&key).ok_or_else(|| {
                    CmdError::not_found(format!("no preference '{key}' (app.prefs lists them)"))
                })?;
                let v = parse_pref(&def, &value).map_err(|e| invalid(format!("{} takes {e}", def.key)))?;
                settings.set_pref(def.key, v);
            }
        }
        settings.save_soon(&editor);
        editor.exec.wake().ping();
    }
    if !key.is_empty() {
        if let Some(v) = named_pref(&settings, &key) {
            return Ok(json!({ "key": key, "value": v }));
        }
        let def = settings
            .def(&key)
            .ok_or_else(|| CmdError::not_found(format!("no preference '{key}' (app.prefs lists them)")))?;
        let mut out = Map::new();
        out.insert("key".into(), json!(def.key));
        out.insert("value".into(), pref_json(&def, settings.pref(def.key).unwrap_or(def.default)));
        if def.ty == PrefType::Angle {
            out.insert("unit".into(), json!("deg"));
        }
        if def.key == "floating_panels" {
            out.insert("note".into(), json!("takes effect after a restart"));
        }
        return Ok(Value::Object(out));
    }
    let mut all = Map::new();
    for k in ["accent", "scale"] {
        if let Some(v) = named_pref(&settings, k) {
            all.insert(k.to_owned(), v);
        }
    }
    for d in PREFS.iter().chain(settings.extra_defs().iter()) {
        all.insert(d.key.to_owned(), pref_json(d, settings.pref(d.key).unwrap_or(d.default)));
    }
    let page = settings.page.lock().to_string();
    Ok(json!({ "preferences": all, "page": page, "angles": "deg" }))
}

// ---- app.keys -----------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum KeysAction {
    /// Every action with its keys, its defaults, its group and the actions it shares a key with.
    #[default]
    List,
    /// Binds `action` to `keys`; another action of an overlapping scope that had them lets go.
    Bind,
    /// Takes `action`'s keys away.
    Clear,
    /// Back to the defaults: `action`'s, or every action's without one.
    Reset,
    /// What `keys` run on the active tab now, found the way a key press is: an action's id, or
    /// nothing.
    Resolve,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct KeysParams {
    /// What to do.
    #[serde(default)]
    action: KeysAction,
    /// The action's id (`edit.undo`, `graph.frame_all`, `level.build`, ...).
    #[serde(default)]
    id: Option<String>,
    /// The keys: "Ctrl+Shift+S", "F5", "Num Del"; several separated by commas.
    #[serde(default)]
    keys: Option<String>,
    /// Only rows whose title, group or keys hold these words (list).
    #[serde(default)]
    query: Option<String>,
}

async fn app_keys(editor: Arc<Editor>, p: KeysParams, _origin: Origin) -> Reply {
    on_ui(&editor, move |shell, ctx| {
        let id = p.id.unwrap_or_default();
        match p.action {
            KeysAction::List => {
                let q = p.query.unwrap_or_default();
                let rows: Vec<Value> = shell
                    .key_rows()
                    .into_iter()
                    .filter(|r| {
                        let hay = format!("{} {} {} {}", r["title"], r["group"], r["id"], r["keys"]);
                        gz_ui::search::matches(&q, &hay)
                    })
                    .collect();
                Ok(json!({ "keys": rows }))
            }
            KeysAction::Bind | KeysAction::Clear => {
                if id.is_empty() {
                    return Err(invalid("id names the action (app.keys lists them)"));
                }
                let chords = if p.action == KeysAction::Bind {
                    let text = p.keys.unwrap_or_default();
                    let chords = parse_keys(&text);
                    if chords.is_empty() {
                        return Err(invalid("keys must be a shortcut such as Ctrl+Shift+S, F5 or Num Del"));
                    }
                    chords
                } else {
                    Vec::new()
                };
                let moved = shell.bind_keys(&id, chords).map_err(CmdError::not_found)?;
                Ok(json!({ "row": shell.key_row(&id), "moved_from": moved }))
            }
            KeysAction::Resolve => {
                let text = p.keys.unwrap_or_default();
                let chord = parse_keys(&text)
                    .first()
                    .copied()
                    .ok_or_else(|| invalid("keys must be a shortcut such as Ctrl+Shift+S, F5 or Num Del"))?;
                Ok(json!({ "keys": chord.label(), "runs": shell.resolve_chord(ctx, &chord), "active_tab": shell.active() }))
            }
            KeysAction::Reset => {
                shell.reset_keys((!id.is_empty()).then_some(id.as_str())).map_err(CmdError::not_found)?;
                Ok(if id.is_empty() { json!({ "reset": "all" }) } else { json!({ "row": shell.key_row(&id) }) })
            }
        }
    })
    .await
}

// ---- doc.* --------------------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DocParams {
    /// The tab's id (`level`, `quest:<path>`, `characters`; `ui.tab` lists them). Left out: the
    /// active tab.
    #[serde(default)]
    doc: Option<String>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReloadParams {
    /// The tab's id. Left out: the active tab.
    #[serde(default)]
    doc: Option<String>,
    /// Drop its unsaved changes and read the file again.
    #[serde(default)]
    discard: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum EditWhat {
    Cut,
    Copy,
    Paste,
    Duplicate,
    Delete,
    Find,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditParams {
    /// What the Edit menu does: cut, copy, paste, duplicate, delete or find.
    action: EditWhat,
    /// The tab's id. Left out: the active tab.
    #[serde(default)]
    doc: Option<String>,
}

fn target(shell: &ShellState, doc: Option<String>) -> Result<String, CmdError> {
    let id = doc.filter(|d| !d.is_empty()).unwrap_or_else(|| shell.active().to_owned());
    if shell.tab_json(&id).is_none() {
        return Err(CmdError::not_found(format!("no tab '{id}' (ui.tab lists them)")));
    }
    Ok(id)
}

async fn doc_list(editor: Arc<Editor>, _p: NoParams, _origin: Origin) -> Reply {
    let docs = on_ui(&editor, |shell, _| Ok(shell.tabs_json())).await?;
    Ok(json!({ "docs": docs, "unsaved": serde_json::to_value(editor.dirty_docs()).unwrap_or(Value::Null) }))
}

async fn doc_save(editor: Arc<Editor>, p: DocParams, origin: Origin) -> Reply {
    let id = on_ui(&editor, move |shell, _| target(shell, p.doc)).await?;
    match crate::unsaved::save_tabs(&editor, vec![id.clone()], origin.may_ask()).await {
        Ok(()) => {}
        // the person gave the save up in the Save dialog: no failure to tell them of
        Err(e) if e.code == crate::unsaved::SAVE_CANCELLED => return Ok(json!({ "cancelled": true })),
        Err(e) => return Err(e),
    }
    let tab = on_ui(&editor, move |shell, _| Ok(shell.tab_json(&id))).await?;
    Ok(json!({ "doc": tab }))
}

fn edit_doc(
    editor: &Arc<Editor>,
    doc: Option<String>,
    what: EditAction,
) -> impl Future<Output = Reply> + use<> {
    let ed = editor.clone();
    async move {
        on_ui(&ed, move |shell, ctx| {
            let id = target(shell, doc)?;
            if !shell.edit_tab(ctx, &id, what, false) {
                return Err(CmdError::new(
                    "refused",
                    format!("the tab '{id}' does not take {}", what.name()),
                ));
            }
            shell.edit_tab(ctx, &id, what, true);
            Ok(json!({ "doc": id, "done": what.name() }))
        })
        .await
    }
}

async fn doc_undo(editor: Arc<Editor>, p: DocParams, _origin: Origin) -> Reply {
    edit_doc(&editor, p.doc, EditAction::Undo).await
}

async fn doc_redo(editor: Arc<Editor>, p: DocParams, _origin: Origin) -> Reply {
    edit_doc(&editor, p.doc, EditAction::Redo).await
}

async fn doc_edit(editor: Arc<Editor>, p: EditParams, _origin: Origin) -> Reply {
    let what = match p.action {
        EditWhat::Cut => EditAction::Cut,
        EditWhat::Copy => EditAction::Copy,
        EditWhat::Paste => EditAction::Paste,
        EditWhat::Duplicate => EditAction::Duplicate,
        EditWhat::Delete => EditAction::Delete,
        EditWhat::Find => EditAction::Find,
    };
    edit_doc(&editor, p.doc, what).await
}

async fn doc_reload(editor: Arc<Editor>, p: ReloadParams, _origin: Origin) -> Reply {
    on_ui(&editor, move |shell, ctx| {
        let id = target(shell, p.doc)?;
        let dirty =
            shell.tab_json(&id).and_then(|t| t.get("dirty").and_then(Value::as_bool)).unwrap_or(false);
        if dirty && !p.discard {
            let label = shell
                .tab_json(&id)
                .and_then(|t| t.get("label").and_then(Value::as_str).map(str::to_owned))
                .unwrap_or_default();
            return Err(unsaved(&[(id, label)]));
        }
        shell.reload_tab(ctx, &id, p.discard).map_err(|e| CmdError::new("refused", e))?;
        Ok(json!({ "doc": id, "reloaded": true }))
    })
    .await
}

// ---- perf.stalls --------------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StallsParams {
    /// Empty the list after answering it.
    #[serde(default)]
    clear: bool,
}

fn perf_stalls(editor: &Editor, p: &StallsParams, _origin: Origin) -> Reply {
    let s = crate::stalls::Stalls::of(editor);
    let out = json!({ "stalls": s.list(), "worst_ms": (s.worst_ms() * 10.0).round() / 10.0,
                      "limit_ms": crate::stalls::STALL_MS, "starved": s.starved() });
    if p.clear {
        s.clear();
    }
    Ok(out)
}

/// Registers `state`, `app.*`, `doc.*` and `perf.stalls`.
pub(crate) fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::read("state", "What is open, what runs and what is wrong: one call to get oriented."),
        state,
    );
    reg.area("app", "The program: its window, quitting, preferences, keys");
    reg.cmd(
        Spec::view(
            "app.window",
            "Minimises, restores, maximises, resizes, moves or focuses the window, or says where it is.",
        )
        .example("action=resize width=1280 height=720"),
        app_window,
    );
    reg.cmd(
        Spec::write("app.quit", "Ends the editor; unsaved work is saved with save, dropped with discard.")
            .destroys_with(&["discard"]),
        app_quit,
    );
    reg.cmd(
        Spec::write(
            "app.switch_project",
            "Closes the project and goes back to the Start page; unsaved work - the scene, the tabs - needs save or discard.",
        )
        .destroys_with(&["discard"]),
        app_switch_project,
    );
    reg.cmd(
        Spec::read("app.about", "The editor's name and version; the window shows them in the About note."),
        app_about,
    );
    reg.cmd(
        Spec::write(
            "app.prefs",
            "Reads or sets a preference (applied at once, saved), or opens Preferences on a page.",
        )
        .example("action=set key=scale value=125"),
        app_prefs,
    );
    reg.cmd(
        Spec::write("app.keys", "Lists the actions with their keys, binds, clears or resets them.")
            .example("action=bind id=graph.frame_all keys=Home"),
        app_keys,
    );
    reg.area("doc", "Any open document: list, save, undo, redo, reload, the Edit menu");
    reg.cmd(
        Spec::read(
            "doc.list",
            "The open documents, which of them hold unsaved work, and every area's unsaved work.",
        ),
        doc_list,
    );
    reg.cmd(Spec::write("doc.save", "Saves an open document (the active tab without doc)."), doc_save);
    reg.cmd(Spec::write("doc.undo", "Takes back the last change of an open document."), doc_undo);
    reg.cmd(Spec::write("doc.redo", "Does again what Undo took back in an open document."), doc_redo);
    reg.cmd(
        Spec::write(
            "doc.edit",
            "Runs the Edit menu's cut, copy, paste, duplicate, delete or find on an open document.",
        ),
        doc_edit,
    );
    reg.cmd(
        Spec::write("doc.reload", "Reads an open document's file again; unsaved changes need discard.")
            .destroys_with(&["discard"]),
        doc_reload,
    );
    reg.area("perf", "How the editor performs: frame pacing, UI-thread stalls");
    reg.cmd_sync(
        Spec::view("perf.stalls", "The UI passes and events that held the UI thread longer than 100 ms."),
        perf_stalls,
    );
}
