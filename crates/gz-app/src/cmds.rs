//! `ui.*`: the window over MCP and the shell (docs/UI_DESIGN.md, section 7; docs/COMMANDS.md).
//!
//! Lengths are points both ways; a rect is `[x, y, w, h]` in pt, and an answer that holds one
//! says the window's `scale` (pixels per point). A command that needs the shell runs a closure on
//! the UI thread ([`on_ui`]); one whose answer describes what is drawn waits for fresh frames
//! first (`FrameClock::after`). Without a window they answer `no_window`.

use std::sync::Arc;

use egui::{Context, Rect};
use gz_editor::{CmdError, Editor, NoParams, Origin, Registry, Reply, Spec, Value, json};
use gz_ui::dialog::{self, Asker};
use gz_ui::{Tone, audit, layout};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Map;

use crate::contract;
use crate::doc::{Drawer, OpenRequest, PanelOp};
use crate::queue::on_ui;
use crate::scroll::{self, ScrollTo};
use crate::shell::{CloseMode, OpenError, ShellState};

pub(crate) fn invalid(text: impl Into<String>) -> CmdError {
    CmdError::invalid(text)
}

/// A length in pt, to a tenth.
pub(crate) fn pt(v: f32) -> f64 {
    (f64::from(v) * 10.0).round() / 10.0
}

/// `[x, y, w, h]` in pt.
pub(crate) fn rect_pt(r: Rect) -> Value {
    json!([pt(r.min.x), pt(r.min.y), pt(r.width()), pt(r.height())])
}

/// `[x, y, w, h]` in pt of a rect given in px as `[x0, y0, x1, y1]`.
pub(crate) fn px_to_pt(r: [f32; 4], ppp: f32) -> Value {
    let s = ppp.max(0.01);
    json!([pt(r[0] / s), pt(r[1] / s), pt((r[2] - r[0]) / s), pt((r[3] - r[1]) / s)])
}

pub(crate) fn scale(ppp: f32) -> f64 {
    (f64::from(ppp) * 1000.0).round() / 1000.0
}

pub(crate) fn asker_of(origin: Origin) -> Asker {
    match origin {
        Origin::Ui => Asker::Ui,
        Origin::Unattended => Asker::Unattended,
        Origin::Mcp => Asker::Mcp,
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
    out.insert("active_tab".into(), json!(shell.active()));
    Value::Object(out)
}

pub(crate) fn tabs_fields(shell: &ShellState) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("active_tab".into(), json!(shell.active()));
    out.insert("tabs".into(), Value::Array(shell.tabs_json()));
    out
}

pub(crate) fn dirty_json(list: &[(String, String)]) -> Value {
    Value::Array(list.iter().map(|(id, label)| json!({ "id": id, "label": label })).collect())
}

pub(crate) fn unsaved(list: &[(String, String)]) -> CmdError {
    CmdError::unsaved(dirty_json(list))
        .with_data(json!({ "dirty": dirty_json(list), "note": "pass save=true or discard=true" }))
}

/// The modal on top: a kit dialog, else the palette. `(title, id, rect pt)`.
pub(crate) fn top_modal(shell: &ShellState, ctx: &Context) -> Option<(String, String, Rect)> {
    if let Some(d) = dialog::modal_on_top(ctx) {
        let ppp = ctx.pixels_per_point();
        let r = Rect::from_min_max(
            egui::pos2(d.rect_px[0] / ppp, d.rect_px[1] / ppp),
            egui::pos2(d.rect_px[2] / ppp, d.rect_px[3] / ppp),
        );
        return Some((d.title, dialog_mcp_id(&d.id), r));
    }
    shell
        .palette_open()
        .then(|| ("Command Palette".to_owned(), contract::PALETTE_ID.to_owned(), shell.palette_rect()))
}

/// The MCP id a kit dialog answers to: the contract's id for one of its dialogs, else
/// `dlg.<name>`.
pub(crate) fn dialog_mcp_id(kit: &str) -> String {
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
            json!({ "id": dialog_mcp_id(&d.id), "title": d.title, "window": format!("{}###{}", d.title, d.id), "rect": px_to_pt(d.rect_px, ppp) })
        })
        .collect();
    if shell.palette_open() {
        out.push(json!({ "id": contract::PALETTE_ID, "title": "Command Palette", "window": contract::PALETTE_WINDOW, "rect": rect_pt(shell.palette_rect()) }));
    }
    out
}

// ---- ui.list ----------------------------------------------------------------------------------

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
            // a panel says where it is in px
            (true, Some([x, y, w, h])) => json!([pt(x / ppp), pt(y / ppp), pt(w / ppp), pt(h / ppp)]),
            _ => Value::Null,
        },
    })
}

async fn ui_list(editor: Arc<Editor>, _p: NoParams, _origin: Origin) -> Reply {
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
            .map(|(id, w)| json!({ "id": id, "window": w, "open": shell.drawer.map(Drawer::id) == Some(*id) }))
            .collect();
        out.insert("drawers".into(), Value::Array(drawers));
        out.insert("dialogs".into(), Value::Array(dialogs_json(shell, ctx)));
        let popups: Vec<Value> = shell
            .popups()
            .iter()
            .map(|p| json!({ "id": p.id, "title": p.title, "window": p.window, "rect": rect_pt(p.rect), "label_x": p.label_x.iter().map(|x| pt(*x)).collect::<Vec<_>>() }))
            .collect();
        out.insert("popups".into(), Value::Array(popups));
        out.insert("work_area".into(), rect_pt(shell.work()));
        out.insert("toasts".into(), json!(gz_ui::state::toast_texts(ctx)));
        out.insert("toast_notes".into(), toasts_json(ctx));
        out.insert("start_page".into(), json!(shell.start_page()));
        // the status bar's centre: the active document's status, none on the Start page
        out.insert("status_centre".into(), shell.status_centre.map_or(Value::Null, rect_pt));
        out.insert("size_class".into(), json!(shell.size_class()));
        let level = shell.level_state();
        out.insert("layout_custom".into(), json!(level.get("layout_custom").and_then(Value::as_bool).unwrap_or(false)));
        out.insert(
            "viewport_maximized".into(),
            json!(level.get("viewport_maximized").and_then(Value::as_bool).unwrap_or(false)),
        );
        out.insert("scale".into(), json!(scale(ppp)));
        Ok(Value::Object(out))
    })
    .await
}

// ---- ui.tab -----------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum TabAction {
    /// The open tabs.
    #[default]
    List,
    /// Brings a tab up.
    Activate,
    /// Opens a document (or brings up the one of that id).
    Open,
    /// Closes a tab: unsaved changes need `save` or `discard`.
    Close,
    /// The next tab, as Ctrl+Tab.
    Next,
    /// The previous tab.
    Prev,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TabParams {
    /// What to do.
    #[serde(default)]
    action: TabAction,
    /// The tab's id: `level`, `quest:<path>`, `characters`, ... (`ui.tab` lists them). Left out:
    /// the active tab.
    #[serde(default)]
    doc: Option<String>,
    /// What to open: one of `ui.contract`'s document kinds.
    #[serde(default)]
    kind: Option<String>,
    /// The file to open, project-relative (a model: meshes-relative).
    #[serde(default)]
    path: Option<String>,
    /// Where a texture is read from: project, library or game.
    #[serde(default)]
    source: Option<String>,
    /// Save unsaved changes first (closing, or a singleton that switches to another file).
    #[serde(default)]
    save: bool,
    /// Drop unsaved changes.
    #[serde(default)]
    discard: bool,
}

/// What `ui.tab action=open` opens: "quest:<rel>" and the singletons' ids name a kind and a
/// path, which `kind=` and `path=` name outright.
fn open_target(id: &str, kind: Option<&str>, path: Option<&str>) -> (String, String) {
    let (mut kind, mut path) = (kind.unwrap_or_default().to_owned(), path.unwrap_or_default().to_owned());
    if !id.is_empty() {
        match id.split_once(':') {
            Some((k, rest)) => {
                if kind.is_empty() {
                    k.clone_into(&mut kind);
                }
                if path.is_empty() {
                    rest.clone_into(&mut path);
                }
            }
            None if kind.is_empty() => id.clone_into(&mut kind),
            None => {}
        }
    }
    (kind, path)
}

async fn ui_tab(editor: Arc<Editor>, p: TabParams, origin: Origin) -> Reply {
    let id = p.doc.clone().unwrap_or_default();
    let has_project = editor.project_opt().is_some();
    if p.save && p.discard {
        return Err(invalid("save and discard exclude each other"));
    }
    if p.action == TabAction::Close && p.save {
        // the tab's save, waited for until it is clean: a document that writes on the pools has
        // not finished when its save returns
        let wanted = id.clone();
        let target = on_ui(&editor, move |shell, _| {
            Ok(if wanted.is_empty() { shell.active().to_owned() } else { wanted })
        })
        .await?;
        crate::unsaved::save_tabs(&editor, vec![target], origin.may_ask()).await?;
    }
    if p.action == TabAction::Open && p.save {
        // a dirty singleton that switches target is saved first, and its save waited for: a tab
        // that writes on the pools is dirty until its file is written
        let (kind, path) = open_target(&id, p.kind.as_deref(), p.path.as_deref());
        if !kind.is_empty() {
            let target = crate::doc::tab_id(&kind, (!path.is_empty()).then_some(path.as_str())).to_string();
            let there = target.clone();
            if on_ui(&editor, move |shell, _| Ok(shell.tab_json(&there).is_some())).await? {
                crate::unsaved::save_tabs(&editor, vec![target], origin.may_ask()).await?;
            }
        }
    }
    let asker = asker_of(origin);
    let person = origin.may_ask();
    on_ui(&editor, move |shell, ctx| match p.action {
        TabAction::List => Ok(Value::Object(tabs_fields(shell))),
        TabAction::Next | TabAction::Prev => {
            shell.cycle(ctx, if p.action == TabAction::Next { 1 } else { -1 });
            Ok(answer_active(shell))
        }
        TabAction::Activate => {
            if id.is_empty() || shell.tab_json(&id).is_none() {
                return Err(CmdError::not_found(format!("no tab '{id}' (ui.tab lists them)")));
            }
            if !shell.activate(ctx, &id) {
                return Err(CmdError::new("refused", "play in editor holds the Level tab"));
            }
            Ok(answer_active(shell))
        }
        TabAction::Open => {
            if !id.is_empty() && shell.tab_json(&id).is_some() {
                if !shell.activate(ctx, &id) {
                    return Err(CmdError::new("refused", "play in editor holds the Level tab"));
                }
                return Ok(answer_active(shell));
            }
            let (kind, path) = open_target(&id, p.kind.as_deref(), p.path.as_deref());
            let source = p.source.unwrap_or_default();
            if kind.is_empty() {
                return Err(invalid("pass doc=, or kind= (and path= for a file)"));
            }
            match kind.as_str() {
                "quest" | "beh" if path.is_empty() => {
                    return Err(invalid("path= names the graph (project-relative)"));
                }
                "model" if path.is_empty() => {
                    return Err(invalid("path= names the model (a visual, meshes-relative)"));
                }
                "texture" if !matches!(source.as_str(), "" | "project" | "library" | "game") => {
                    return Err(invalid("source must be project, library or game"));
                }
                "mod_settings" | "releases" if !has_project => return Err(CmdError::no_project()),
                _ => {}
            }
            let req = OpenRequest {
                kind,
                path: (!path.is_empty()).then_some(path),
                id: None,
                arg: (!source.is_empty()).then_some(source),
                activate: true,
            };
            match shell.open(ctx, req, p.discard, asker) {
                Ok(()) => Ok(answer_active(shell)),
                Err(OpenError::Unsaved { id, label }) => Err(unsaved(&[(id, label)])),
                Err(OpenError::Other(e)) => Err(invalid(e)),
            }
        }
        TabAction::Close => {
            let id = if id.is_empty() { shell.active().to_owned() } else { id };
            let Some(info) = shell.tab_json(&id) else {
                return Err(CmdError::not_found(format!("no tab '{id}' (ui.tab lists them)")));
            };
            // from the window, a dirty tab asks; the caller over MCP has decided
            if asker == Asker::Ui && !p.save && !p.discard {
                shell.request_close(ctx, &id);
                return Ok(json!({ "closing": id }));
            }
            let mode = if p.save {
                CloseMode::SaveFirst
            } else if p.discard || asker == Asker::Unattended {
                CloseMode::Discard
            } else {
                CloseMode::IfClean
            };
            match shell.close_tab(ctx, &id, mode, person) {
                Ok(()) => {
                    let mut out = Map::new();
                    out.insert("closed".into(), json!(id));
                    Ok(merge(out, Value::Object(tabs_fields(shell))))
                }
                Err(e) if e == "unsaved" => {
                    let label = info.get("label").and_then(Value::as_str).unwrap_or_default().to_owned();
                    Err(unsaved(&[(id, label)]))
                }
                Err(e) => Err(CmdError::new("refused", e)),
            }
        }
    })
    .await
}

// ---- ui.panel ---------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PanelAction {
    Open,
    Close,
    #[default]
    Focus,
    Toggle,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PanelParams {
    /// A panel of the Level tab: place, world_outliner, details, level_settings, content_browser,
    /// output_log, problems or viewport (`ui.contract` lists them).
    id: String,
    /// What to do with it.
    #[serde(default)]
    action: PanelAction,
    /// true moves it into a window of its own, false docks it again (Preferences > General >
    /// Allow Panels Outside the Window).
    #[serde(default)]
    floating: Option<bool>,
}

async fn ui_panel(editor: Arc<Editor>, p: PanelParams, _origin: Origin) -> Reply {
    let moved = p.floating.is_some();
    let answer = on_ui(&editor, move |shell, ctx| {
        let id = p.id;
        if !shell.has_panel(&id) {
            let ids: Vec<&str> = contract::PANELS.iter().map(|(id, _)| *id).collect();
            return Err(invalid(format!("no panel '{id}': the panels are {}", ids.join(", "))));
        }
        let op = match (p.floating, p.action) {
            (Some(true), _) => PanelOp::Float,
            (Some(false), _) => PanelOp::Dock,
            (None, PanelAction::Open) => PanelOp::Open,
            (None, PanelAction::Close) => PanelOp::Close,
            (None, PanelAction::Focus) => PanelOp::Focus,
            (None, PanelAction::Toggle) => PanelOp::Toggle,
        };
        if p.floating.is_some() && id == "viewport" {
            return Err(CmdError::new("refused", "the viewport stays in the editor's window"));
        }
        if op == PanelOp::Float && !crate::shell::floating_panels(ctx) {
            return Err(CmdError::new(
                "refused",
                "panels stay in the window: Preferences > General > Allow Panels Outside the Window is off (it takes a restart)",
            ));
        }
        let closing =
            op == PanelOp::Close || op == PanelOp::Toggle && shell.level_panel(&id).is_some_and(|p| p.open);
        if closing && id == "viewport" {
            return Err(CmdError::new(
                "refused",
                "the viewport stays on Level: F11 (View > Maximize Viewport) hides the panels",
            ));
        }
        let host = shell.panel(ctx, &id, op).ok_or_else(|| CmdError::new("failed", format!("the {id} panel did not answer")))?;
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

// ---- ui.drawer --------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum DrawerAction {
    Open,
    Close,
    /// Open when closed, closed when open (Ctrl+Space, `).
    Toggle,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DrawerParams {
    /// What to do.
    action: DrawerAction,
    /// content or log. Left out with close: whichever is up.
    #[serde(default)]
    id: Option<String>,
}

async fn ui_drawer(editor: Arc<Editor>, p: DrawerParams, _origin: Origin) -> Reply {
    on_ui(&editor, move |shell, ctx| {
        let which = p.id.as_deref().and_then(Drawer::parse);
        if p.id.is_some() && which.is_none() {
            return Err(invalid("id must be content or log"));
        }
        let host = match p.action {
            DrawerAction::Open | DrawerAction::Toggle => {
                let Some(which) = which else { return Err(invalid("id must be content or log")) };
                if !shell.has_drawer(which) {
                    return Err(CmdError::not_found(format!(
                        "no {} drawer is installed in this shell",
                        which.id()
                    )));
                }
                if shell.start_page() {
                    return Err(CmdError::new(
                        "refused",
                        "the Start page has no drawers: open a project first",
                    ));
                }
                if p.action == DrawerAction::Toggle || shell.drawer_panel_visible(which) {
                    shell.toggle_drawer(ctx, which);
                } else {
                    shell.open_drawer(which);
                }
                if shell.drawer == Some(which) {
                    "drawer"
                } else if shell.drawer_panel_visible(which) {
                    "panel"
                } else {
                    "none"
                }
            }
            DrawerAction::Close => {
                if which.is_none() || shell.drawer == which {
                    shell.drawer = None;
                }
                "none"
            }
        };
        Ok(json!({ "drawer": shell.drawer.map_or("none", Drawer::id), "host": host }))
    })
    .await
}

// ---- ui.dialog --------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum DialogAction {
    /// The dialogs up and the ones that open by id.
    #[default]
    List,
    Open,
    /// As Esc.
    Close,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DialogParams {
    /// What to do.
    #[serde(default)]
    action: DialogAction,
    /// A dialog of `ui.contract` (`package_release`, `publish_release`, ...), or a kit dialog's
    /// `dlg.<name>` to close. Left out with close: the dialog on top.
    #[serde(default)]
    id: Option<String>,
}

async fn ui_dialog(editor: Arc<Editor>, p: DialogParams, _origin: Origin) -> Reply {
    on_ui(&editor, move |shell, ctx| {
        if p.action == DialogAction::List {
            let known: Vec<&str> = contract::DIALOGS.iter().map(|(i, _)| *i).collect();
            return Ok(json!({ "dialogs": dialogs_json(shell, ctx), "known": known }));
        }
        let id = p.id.unwrap_or_default();
        // close without an id: the dialog on top, as Esc closes it
        let id = if id.is_empty() && p.action == DialogAction::Close {
            if shell.palette_open() {
                contract::PALETTE_ID.to_owned()
            } else {
                match dialog::modal_on_top(ctx) {
                    Some(d) => dialog_mcp_id(&d.id),
                    None => return Err(CmdError::not_found("no dialog is open (ui.dialog lists what is)")),
                }
            }
        } else {
            id
        };
        if id.is_empty() {
            return Err(invalid("id names the dialog (ui.contract lists them)"));
        }
        if p.action == DialogAction::Open {
            shell.open_dialog(ctx, &id).map_err(|e| CmdError::new("refused", e))?;
            let window = contract::window_of_known(&id).map_or_else(|| format!("###dlg.{id}"), str::to_owned);
            return Ok(json!({ "id": id, "window": window }));
        }
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
                return Err(CmdError::not_found(format!("'{id}' is not open (ui.dialog lists what is)")));
            }
        }
        Ok(json!({ "closing": id }))
    })
    .await
}

// ---- ui.popup, ui.toast ------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PopupParams {
    /// A popup of `ui.contract` (`menu:<name>`, `tab_menu:<tab>`, `tasks`, ...), or none to take
    /// the one up down. To look at, never to act.
    show: String,
}

async fn ui_popup(editor: Arc<Editor>, p: PopupParams, _origin: Origin) -> Reply {
    on_ui(&editor, move |shell, ctx| {
        shell.show_popup(ctx, &p.show).map_err(|e| CmdError::new("refused", e))?;
        Ok(json!({ "popup": p.show }))
    })
    .await
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ToastParams {
    /// The sentence.
    text: String,
    /// info, success, warning or error.
    #[serde(default)]
    tone: Option<String>,
    /// The note's one button: the id of the action it runs (`app.keys` lists them).
    #[serde(default)]
    action: Option<String>,
    /// The button's label (with `action`; its title without).
    #[serde(default)]
    label: Option<String>,
}

/// The notes up now, each with its button.
fn toasts_json(ctx: &Context) -> Value {
    Value::Array(
        gz_ui::state::toast_list(ctx)
            .into_iter()
            .map(|(text, button)| match button {
                Some((label, action)) => json!({ "text": text, "button": label, "action": action }),
                None => json!({ "text": text }),
            })
            .collect(),
    )
}

async fn ui_toast(editor: Arc<Editor>, p: ToastParams, _origin: Origin) -> Reply {
    let tone = match p.tone.as_deref().unwrap_or("info") {
        "info" => Tone::Info,
        "success" => Tone::Success,
        "warning" => Tone::Warning,
        "error" => Tone::Error,
        _ => return Err(invalid("tone must be info, success, warning or error")),
    };
    on_ui(&editor, move |shell, ctx| {
        match p.action.as_deref() {
            Some(id) => {
                let title = shell
                    .action(id)
                    .map(|a| a.title)
                    .ok_or_else(|| CmdError::not_found(format!("no action '{id}' (app.keys lists them)")))?;
                // the caller's words are its own; an action's title is the editor's
                let label = match p.label.as_deref() {
                    Some(l) => gz_ui::Tx::raw(l.trim_end_matches("...").to_owned()),
                    None => gz_ui::Tx::from(title),
                };
                gz_ui::state::toast_with(ctx, tone, gz_ui::Tx::raw(p.text.clone()), label, id);
            }
            None => gz_ui::state::toast(ctx, tone, gz_ui::Tx::raw(p.text.clone())),
        }
        Ok(json!({ "toasts": gz_ui::state::toast_texts(ctx), "toast_notes": toasts_json(ctx) }))
    })
    .await
}

// ---- ui.scroll, ui.split, ui.audit, ui.scale, ui.contract --------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScrollParams {
    /// A panel, a drawer, a dialog, `work_area`, or a window's own name.
    id: String,
    /// top, bottom, a number of pt from the top, or item:<name>.
    to: String,
}

async fn ui_scroll(editor: Arc<Editor>, p: ScrollParams, _origin: Origin) -> Reply {
    let target = ScrollTo::parse(&p.to)
        .ok_or_else(|| invalid("to must be top, bottom, a number of pt or item:<name>"))?;
    let id = p.id.clone();
    let window = on_ui(&editor, move |shell, ctx| {
        let window = shell.window_of(&id);
        if !matches!(target, ScrollTo::Item(_)) && scroll::last_scroll(ctx, &window).is_none() {
            return Err(CmdError::not_found(format!("'{id}' is not on screen")));
        }
        scroll::request_scroll(ctx, &window, target);
        ctx.request_repaint();
        Ok(window)
    })
    .await?;
    // where it landed: the frames that applied the request, then what the surface reports
    editor.frames.after(2).await?;
    on_ui(&editor, move |_, ctx| {
        let (y, max) = scroll::last_scroll(ctx, &window).unwrap_or((0.0, 0.0));
        Ok(json!({ "id": p.id, "window": window, "scroll_y": pt(y), "scroll_max_y": pt(max) }))
    })
    .await
}

fn number(v: f32) -> Value {
    let s: f64 = format!("{v}").parse().unwrap_or(f64::from(v));
    if s.fract() == 0.0 { json!(s as i64) } else { json!(s) }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SplitParams {
    /// A key of the layout store (`graph.panel_w`, `level.*`). Left out: every key.
    #[serde(default)]
    key: Option<String>,
    /// Its new value in pt (or a share, for a split that is one).
    #[serde(default)]
    value: Option<f32>,
}

async fn ui_split(editor: Arc<Editor>, p: SplitParams, _origin: Origin) -> Reply {
    on_ui(&editor, move |_, ctx| {
        let Some(key) = p.key.filter(|k| !k.is_empty()) else {
            let mut splits = Map::new();
            for (k, v) in layout::all(ctx) {
                splits.insert(k.to_string(), number(v));
            }
            return Ok(json!({ "splits": splits, "unit": "pt" }));
        };
        match p.value {
            Some(v) => layout::set(ctx, &key, v),
            None if !layout::has(ctx, &key) => {
                return Err(CmdError::not_found(format!(
                    "no split '{key}' (ui.split with no key lists them)"
                )));
            }
            None => {}
        }
        Ok(json!({ "key": key, "value": number(layout::get(ctx, &key, 0.0)), "unit": "pt" }))
    })
    .await
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AuditParams {
    /// Only what this panel, drawer, dialog or window holds.
    #[serde(default)]
    id: Option<String>,
}

async fn ui_audit(editor: Arc<Editor>, p: AuditParams, _origin: Origin) -> Reply {
    on_ui(&editor, |_, ctx| {
        audit::begin(ctx);
        ctx.request_repaint();
        Ok(())
    })
    .await?;
    editor.frames.after(2).await?;
    on_ui(&editor, move |shell, ctx| {
        let entries = audit::take(ctx);
        let window = p.id.filter(|i| !i.is_empty()).map(|i| shell.window_of(&i));
        let ppp = ctx.pixels_per_point();
        let keep = |surface: &str| window.as_ref().is_none_or(|w| surface == w || surface.ends_with(w.as_str()));
        let (mut truncated, mut escaped, mut controls, mut loading, mut errors) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut untranslated: Vec<Value> = Vec::new();
        for e in entries {
            match e {
                audit::AuditEntry::Truncated { surface, full, shown, en, rect } if keep(&surface) => {
                    let mut t = json!({ "window": surface, "label": full, "shown": shown, "rect": px_to_pt(rect, ppp) });
                    if let (Some(en), Some(o)) = (en, t.as_object_mut()) {
                        o.insert("en".into(), json!(en));
                    }
                    truncated.push(t);
                }
                audit::AuditEntry::Untranslated { surface, text } if keep(&surface) => {
                    let row = json!({ "window": surface, "label": text });
                    if !untranslated.contains(&row) {
                        untranslated.push(row);
                    }
                }
                audit::AuditEntry::Escaped { surface, what, rect } if keep(&surface) => {
                    escaped.push(json!({ "window": surface, "label": what, "rect": px_to_pt(rect, ppp) }));
                }
                audit::AuditEntry::Control { surface, label, enabled, why, help, rect } if keep(&surface) => {
                    controls.push(json!({ "window": surface, "label": label, "enabled": enabled, "why": why.unwrap_or_default(), "help": help.unwrap_or_default(), "rect": px_to_pt(rect, ppp) }));
                }
                audit::AuditEntry::Loading { surface, what, rect } if keep(&surface) => {
                    loading.push(json!({ "window": surface, "label": what, "rect": px_to_pt(rect, ppp) }));
                }
                audit::AuditEntry::Error { surface, title, text, rect } if keep(&surface) => {
                    errors.push(json!({ "window": surface, "label": title, "text": text, "rect": px_to_pt(rect, ppp) }));
                }
                _ => {}
            }
        }
        Ok(json!({
            "truncated": truncated,
            "escaped": escaped,
            "controls": controls,
            "loading": loading,
            "errors": errors,
            "language": gz_ui::i18n::lang(ctx).key(),
            "untranslated": untranslated,
            "scale": scale(ppp),
        }))
    })
    .await
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ScaleParams {
    /// The UI at this scale (1.0 .. 2.5) until the editor ends; never saved (Preferences >
    /// Interface > Interface Scale is the author's). Left out: the scale now.
    #[serde(default)]
    value: Option<f32>,
}

async fn ui_scale(editor: Arc<Editor>, p: ScaleParams, _origin: Origin) -> Reply {
    if let Some(v) = p.value
        && !(contract::UI_SCALE_MIN..=contract::UI_SCALE_MAX).contains(&v)
    {
        return Err(invalid(format!("value must be {}..{}", contract::UI_SCALE_MIN, contract::UI_SCALE_MAX)));
    }
    on_ui(&editor, move |shell, ctx| {
        if let Some(v) = p.value {
            shell.test_scale = Some(v);
            let native = ctx.native_pixels_per_point().unwrap_or(1.0).max(0.1);
            ctx.set_zoom_factor(v / native);
            ctx.request_repaint();
        }
        Ok(json!({ "scale": scale(p.value.unwrap_or_else(|| ctx.pixels_per_point())), "saved": false }))
    })
    .await?;
    editor.frames.after(2).await?;
    on_ui(&editor, |_, ctx| Ok(json!({ "scale": scale(ctx.pixels_per_point()), "saved": false }))).await
}

pub(crate) fn ui_contract_json() -> Value {
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
        "themes": gz_ui::THEMES.iter().map(|t| t.id).collect::<Vec<_>>(),
        "categories": contract::CATEGORIES,
        "card_menu": [contract::CARD_SHOW_IN_EXPLORER, contract::CARD_DELETE, contract::CARD_REMOVE_FROM_LIST],
        "delete_project_dialog": contract::DELETE_PROJECT_DIALOG,
        "panels": pairs(&contract::PANELS),
        "drawers": pairs(&contract::DRAWERS),
        "dialogs": pairs(&contract::DIALOGS),
        "document_kinds": contract::DOCUMENT_KINDS,
        "popups": contract::POPUPS,
        "view_popups": contract::VIEW_POPUPS,
        "menus": contract::MENUS,
        "viewport_menus": contract::VIEWPORT_MENUS,
        "kit_dialogs": contract::KIT_DIALOGS,
        "palette": contract::PALETTE_ID,
        "task_card": contract::TASK_CARD,
        "ui_scale_range": [contract::UI_SCALE_MIN, contract::UI_SCALE_MAX],
        "interface_scales": contract::INTERFACE_SCALES,
        "stall_ms": crate::stalls::STALL_MS,
    })
}

// ---- ui.frame, ui.reset_layout, ui.reveal --------------------------------------------------------

async fn ui_frame(editor: Arc<Editor>, _p: NoParams, _origin: Origin) -> Reply {
    editor.frames.after(2).await?;
    on_ui(&editor, |shell, ctx| {
        let c = shell.caption_geom();
        let tabs: Vec<Value> = shell
            .tab_rects()
            .into_iter()
            .map(|(id, r, icon_only)| json!({ "id": id, "rect": rect_pt(r), "icon_only": icon_only }))
            .collect();
        let maximized = ctx.input(|i| i.viewport().maximized == Some(true));
        Ok(json!({
            "band": rect_pt(c.bar),
            "menus_end": pt(c.menus_end),
            "buttons_x": pt(c.buttons_x),
            "maximized": maximized,
            "icon": shell.has_logo(),
            "caption_h": pt(c.bar.height()),
            "scale": scale(ctx.pixels_per_point()),
            "folded": shell.folded(),
            "interactive": [[pt(c.menus_x0), pt(c.menus_end)], [pt(c.buttons_x), pt(c.bar.max.x)]],
            "tab_row": if c.tab_row.is_positive() { rect_pt(c.tab_row) } else { Value::Null },
            "tabs": tabs,
            "toolbar": shell.toolbar_rect.map_or(Value::Null, rect_pt),
            "project_toolbar": shell.project_toolbar_rect.map_or(Value::Null, rect_pt),
            "toolbar_status": shell.toolbar_status_shown.clone(),
            "status_bar": rect_pt(shell.status_rect),
            "status_line": shell.status_line_rect.map_or(Value::Null, rect_pt),
            "status_chips": shell.status_chip_ids.clone(),
        }))
    })
    .await
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ResetLayoutParams {
    /// level (the Level tab's panels, the default) or all (every split and grid width too).
    #[serde(default)]
    scope: Option<String>,
}

async fn ui_reset_layout(editor: Arc<Editor>, p: ResetLayoutParams, _origin: Origin) -> Reply {
    let scope = p.scope.unwrap_or_else(|| "level".to_owned());
    if scope != "level" && scope != "all" {
        return Err(invalid("scope must be level or all"));
    }
    let all = scope == "all";
    on_ui(&editor, move |shell, ctx| {
        shell.reset_layout(ctx, all);
        Ok(())
    })
    .await?;
    Ok(json!({ "scope": scope }))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RevealParams {
    /// A file or folder of the project, project-relative (or an absolute path, which a tab whose
    /// file lies outside the project names). Left out: the project's folder.
    #[serde(default)]
    path: Option<String>,
    /// Answer what would be shown and open nothing.
    #[serde(default)]
    dry_run: bool,
}

async fn ui_reveal(editor: Arc<Editor>, p: RevealParams, _origin: Origin) -> Reply {
    let root = editor.project()?.root.clone();
    let rel = p.path.unwrap_or_default().replace('/', "\\");
    let full = if rel.is_empty() { root.clone() } else { root.join(&rel) };
    let there = full.clone();
    if !editor.exec.io(move || there.exists()).await {
        return Err(CmdError::not_found(format!("{} is not there.", full.display())));
    }
    if !p.dry_run {
        let shown = full.clone();
        let is_dir = rel.is_empty();
        editor
            .exec
            .io(move || {
                if is_dir || shown.is_dir() {
                    crate::chrome::reveal_folder(&shown);
                } else {
                    crate::chrome::reveal_file(&shown);
                }
            })
            .await;
    }
    Ok(json!({ "shown": full.display().to_string(), "opened": !p.dry_run }))
}

// ---- ui.palette ---------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PaletteAction {
    /// Whether it is open, and its rows for the query.
    #[default]
    State,
    /// Its rows for a query, ranked.
    Query,
    Open,
    Close,
    /// Runs a row by its id: an action's, `tab:<id>` or `panel:<id>`.
    Run,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PaletteParams {
    /// What to do.
    #[serde(default)]
    action: PaletteAction,
    /// What to search for: words that must each match a row's name, category or keys.
    #[serde(default)]
    query: Option<String>,
    /// The row to run (run).
    #[serde(default)]
    id: Option<String>,
    /// How many rows to answer (1..250, 50 without).
    #[serde(default)]
    limit: Option<usize>,
}

async fn ui_palette(editor: Arc<Editor>, p: PaletteParams, _origin: Origin) -> Reply {
    let query = p.query.unwrap_or_default();
    let limit = p.limit.unwrap_or(50).clamp(1, 250);
    on_ui(&editor, move |shell, ctx| {
        match p.action {
            PaletteAction::Open => shell.open_palette(&query),
            PaletteAction::Close => shell.close_palette(),
            PaletteAction::Run => {
                let id = p.id.unwrap_or_default();
                if id.is_empty() {
                    return Err(invalid("run takes id= (a row's id, as ui.palette query answers it)"));
                }
                shell.palette_run(ctx, &id).map_err(|e| CmdError::new("refused", e))?;
                shell.close_palette();
                return Ok(json!({ "ran": id }));
            }
            PaletteAction::Query | PaletteAction::State => {}
        }
        let q = if p.action == PaletteAction::State { shell.palette.query.clone() } else { query };
        let rows = shell.palette_ranked(ctx, &q);
        let results: Vec<Value> = rows.iter().take(limit).map(crate::palette::PaletteRow::to_json).collect();
        Ok(json!({ "open": shell.palette_open(), "query": q, "results": results, "total": rows.len() }))
    })
    .await
}

// ---- ui.tasks -----------------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TasksParams {
    /// true puts the Task Center's card up (while a job runs or one has ended), false takes it
    /// down. Left out: as it is.
    #[serde(default)]
    show: Option<bool>,
}

async fn ui_tasks(editor: Arc<Editor>, p: TasksParams, _origin: Origin) -> Reply {
    // read without the UI thread: a caller polling a build is answered at once
    let marks = crate::shell::JobMarks::of(&editor);
    let log = marks.log_state(&editor);
    let now = crate::tasks::now_ms();
    let cancelled: Vec<gz_editor::JobId> = marks.cancelled.lock().iter().copied().collect();
    let jobs = editor.jobs.snapshot();
    let running: Vec<Value> = jobs
        .iter()
        .filter(|j| j.running())
        .map(|j| {
            crate::tasks::task_json(
                j,
                crate::tasks::card_of(&editor, j.id).as_ref(),
                cancelled.contains(&j.id),
                now,
            )
        })
        .collect();
    let finished: Vec<Value> =
        crate::tasks::finished_rows(&editor).iter().map(|j| crate::tasks::finished_json(j, now)).collect();
    // only a change of what the window shows goes to the UI thread; a read never waits for it -
    // not for a frame, and not for a window whose graphics device is still being made
    let mut card = Value::Null;
    if let Ok(q) = crate::queue::UiQueue::of(&editor) {
        card = match p.show {
            Some(show) => {
                q.run(move |shell, _| {
                    shell.show_card(show);
                    Ok(json!(shell.card_open))
                })
                .await?
            }
            None => json!(marks.card_open.load(std::sync::atomic::Ordering::Acquire)),
        };
    }
    Ok(json!({
        "running": running,
        "finished": finished,
        "card_open": card,
        "errors": log.errors,
        "warnings": log.warnings,
        "status_line": log.last,
    }))
}

/// Registers `ui.*`. Call it after the commands it replaces: a name registered twice keeps the
/// later handler.
pub fn register(reg: &mut Registry) {
    reg.area(
        "ui",
        "The window: tabs, panels, drawers, dialogs, popups, scroll, splits, scale, the palette, the audit",
    );
    reg.cmd(
        Spec::read(
            "ui.list",
            "What the window shows: tabs, panels, drawers, dialogs, popups, the work area, the scale.",
        ),
        ui_list,
    );
    reg.cmd(
        Spec::view("ui.tab", "Lists, opens, brings up, cycles or closes the major tabs.")
            .destroys_with(&["discard"])
            .example("action=open kind=characters"),
        ui_tab,
    );
    reg.cmd(
        Spec::view("ui.panel", "Opens, closes, focuses or toggles a panel of the Level tab.")
            .example("action=focus id=details"),
        ui_panel,
    );
    reg.cmd(
        Spec::view("ui.drawer", "Opens, closes or toggles the Content or the Log drawer over any tab.")
            .example("action=open id=log"),
        ui_drawer,
    );
    reg.cmd(
        Spec::view("ui.dialog", "Lists the dialogs, opens one by its id, or closes one as Esc does.")
            .example("action=open id=package_release"),
        ui_dialog,
    );
    reg.cmd(
        Spec::view("ui.popup", "Puts a menu or a popup up to be looked at, or takes it down.")
            .example("show=menu:File"),
        ui_popup,
    );
    reg.cmd(Spec::view("ui.toast", "Shows a note in the window's corner for a few seconds."), ui_toast);
    reg.cmd(
        Spec::view("ui.scroll", "Scrolls a panel, a drawer or a dialog and answers where it stands, in pt.")
            .example("id=details to=bottom"),
        ui_scroll,
    );
    reg.cmd(Spec::view("ui.split", "Lists the layout store's sizes, or sets one by its key."), ui_split);
    reg.cmd(
        Spec::read(
            "ui.audit",
            "What the next frame cut short, let escape or drew off, with the reasons, what it still loads, the error states it shows in place of content and, in another language than English, the editor's own words its catalog lacks.",
        ),
        ui_audit,
    );
    reg.cmd(
        Spec::view("ui.scale", "Sets the UI's scale for tests until the editor ends; never saved."),
        ui_scale,
    );
    reg.cmd_sync(
        Spec::read(
            "ui.contract",
            "The window's constants that tests pin: panels, drawers, dialogs, menus, kinds.",
        ),
        |_, _: &NoParams, _| Ok(ui_contract_json()),
    );
    reg.cmd(
        Spec::read(
            "ui.frame",
            "The frame's geometry: the caption, the tab row and its tabs, the toolbar, the status bar.",
        ),
        ui_frame,
    );
    reg.cmd(
        Spec::view("ui.reset_layout", "Rebuilds the Level tab's layout; scope=all also forgets every split."),
        ui_reset_layout,
    );
    reg.cmd(
        Spec::view(
            "ui.reveal",
            "Shows a file or folder of the project, or the project's folder, in Explorer.",
        ),
        ui_reveal,
    );
    reg.cmd(
        Spec::view(
            "ui.palette",
            "Opens the Command Palette, ranks its rows for a query, or runs a row by its id.",
        )
        .example("action=run id=window.log_drawer"),
        ui_palette,
    );
    reg.cmd(
        Spec::view(
            "ui.tasks",
            "The Task Center as drawn: the jobs that run, the last ones that ended, the counts.",
        ),
        ui_tasks,
    );
    crate::app_cmds::register(reg);
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
        assert_eq!(c["panels"].as_object().map(|d| d.len()), Some(8));
    }

    #[test]
    fn kit_dialogs_answer_to_the_contract_ids() {
        assert_eq!(dialog_mcp_id("dlg.import_base_scene"), "import_base_scene");
        assert_eq!(dialog_mcp_id("dlg.catalog_submit"), "submit");
        assert_eq!(dialog_mcp_id("dlg.delete_project"), "dlg.delete_project");
    }
}
