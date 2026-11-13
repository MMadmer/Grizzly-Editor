//! `pda_preview` (`EditorPda::McpPreview`, frozen wire): open / state / click / page / key /
//! aspect / lang / reset / reload / close / apply / save / revert, with `pane`, `tab` and
//! `zoom` beside any action. The state answers in CANVAS units. An answer that depends on the
//! frame waits for the frames that show it, so `pending` is false.

use std::path::Path;
use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, CmdError, Editor, Reply};
use gz_pda::{Aspect, PdaService, Preview};
use serde_json::{Map, Value, json};

use crate::assets::PdaAssets;
use crate::gamefile::atof;
use crate::pda::canvas::{ZOOM_MAX, ZOOM_MIN, clock_now, item_rects, map_view};
use crate::pda::{Model, PdaTab, TAB_IDS, follow, load_file, read, reload_all, save_text};

fn invalid(text: impl Into<String>) -> CmdError {
    CmdError::invalid(text)
}

/// `ArgString`: present means given, even empty; a bare number or flag is its text.
fn arg(args: &Args, key: &str) -> Option<String> {
    match args.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A string argument the way `ArgString` filled it in: emptied when it is not there.
fn arg_or_empty(args: &Args, key: &str) -> String {
    arg(args, key).unwrap_or_default()
}

/// `ArgBool`: `true` / `1` at the start of the value.
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

/// `ArgInt`: a number (cut toward zero) or text that starts like one; `def` otherwise.
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

/// `ArgFloat`: a number, or text that starts like one.
fn arg_float(args: &Args, key: &str) -> Option<f32> {
    match args.get(key)? {
        Value::Number(n) => n.as_f64().map(|v| v as f32),
        Value::String(s) => {
            let t = s.trim_start_matches([' ', '\t']);
            t.starts_with(|c: char| c == '-' || c == '.' || c.is_ascii_digit()).then(|| atof(t))
        }
        _ => None,
    }
}

/// `%g` as a JSON number.
fn g(v: f32) -> Value {
    let text = gz_luadata::format_g(f64::from(v), 6);
    match text.parse::<f64>() {
        Ok(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => json!(n as i64),
        Ok(n) => json!(n),
        Err(_) => json!(0),
    }
}

/// `%.0f` as a JSON number.
fn whole(v: f32) -> Value {
    if v.is_finite() { json!(v.round() as i64) } else { json!(0) }
}

/// `%.<d>f` as a JSON number.
fn fixed(v: f32, decimals: i32) -> Value {
    let k = 10f64.powi(decimals);
    if v.is_finite() { json!((f64::from(v) * k).round() / k) } else { json!(0) }
}

/// The Source pane's unsaved edits go only with `save=` or `discard=` (docs/UI_DESIGN.md,
/// section 4, precedence).
async fn settle(editor: &Arc<Editor>, tab: &Arc<PdaTab>, save: bool, discard: bool) -> Result<(), CmdError> {
    let (dirty, rel, path) = {
        let m = tab.m.lock();
        (m.text_dirty, m.rel.clone(), m.path.clone())
    };
    if !dirty {
        return Ok(());
    }
    if save {
        return save_text(editor, tab).await.map_err(invalid);
    }
    if discard {
        load_file(editor, tab, &path).await;
        return Ok(());
    }
    Err(CmdError::unsaved(json!([rel])))
}

/// Runs a `ui.tab` action, as the window would; headless there is no tab to touch.
async fn ui_tab(editor: &Arc<Editor>, fields: &[(&str, Value)]) -> Value {
    let mut args = Args::new();
    for (k, v) in fields {
        args.insert((*k).to_owned(), v.clone());
    }
    editor.call("ui.tab", args, Origin::Mcp).await
}

fn click(m: &mut Model, args: &Args) -> Result<(), CmdError> {
    // by id, by the n of the state's `visible` list, or by a canvas point
    let boxes = m.preview.boxes();
    let id = arg_or_empty(args, "id");
    let target = if !id.is_empty() {
        let row = u32::try_from(arg_int(args, "row", 0).max(0)).unwrap_or(0);
        m.preview.target_by_id(&boxes, &id, row)
    } else if arg_int(args, "n", 0) > 0 {
        Preview::target_by_n(&boxes, usize::try_from(arg_int(args, "n", 0)).unwrap_or(0))
    } else {
        let (Some(x), Some(y)) = (arg_float(args, "x"), arg_float(args, "y")) else {
            return Err(invalid("click needs 'id', 'n' or a canvas point 'x','y'"));
        };
        m.preview.target_at(&boxes, x, y)
    }
    .map_err(invalid)?;
    if m.preview.closed {
        m.preview.closed = false;
    }
    m.preview.click(target, &clock_now());
    Ok(())
}

pub async fn pda_preview(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let session = editor.project()?;
    let tab = PdaTab::of_session(&editor, &session);
    let assets = PdaAssets::of_session(&session);
    // the second read of a pending answer, once its frames are drawn: it applies nothing
    if arg_bool(&args, "answer_only") {
        let (active, _) = shown(&editor).await;
        return Ok(answer(&editor, &tab, &assets, false, active).await);
    }
    let action = arg(&args, "action").filter(|a| !a.is_empty()).unwrap_or_else(|| "state".to_owned());
    let mut path = arg_or_empty(&args, "path");
    let given_path = !path.is_empty();
    let (save, discard) = (arg_bool(&args, "save"), arg_bool(&args, "discard"));
    // a file written behind the preview is read again before anything looks at it
    follow(&editor, &tab).await;

    let (current, dirty) = {
        let m = tab.m.lock();
        (m.path.clone(), m.text_dirty)
    };
    if given_path || current.is_empty() || action == "open" {
        if path.is_empty() {
            path = if current.is_empty() {
                PdaService::of_session(&editor, &session).used_layout(&editor).await
            } else {
                current.clone()
            };
        }
        if path.is_empty() {
            return Err(invalid("missing 'path' (and the module uses no PDA layout yet)"));
        }
        let abs = gz_project::ops::resolve_in_project(&tab.root, &path).map_err(invalid)?.replace('/', "\\");
        let other = !abs.eq_ignore_ascii_case(&current);
        // another file is read after the unsaved edits are settled; the same one opened again keeps them
        if other {
            settle(&editor, &tab, save, discard).await?;
        }
        let dirty_now = tab.m.lock().text_dirty;
        if (other || (action == "open" && !dirty_now)) && !load_file(&editor, &tab, &abs).await {
            return Err(invalid(format!("{path} cannot be read")));
        }
    }
    let _ = dirty;

    match action.as_str() {
        // open brings the pda tab up at once; state and the rest never switch tabs
        "open" => {
            let rel = {
                let mut m = tab.m.lock();
                m.open = true;
                m.rel.clone()
            };
            let _ = ui_tab(
                &editor,
                &[
                    ("action", json!("open")),
                    ("kind", json!("pda")),
                    ("path", json!(rel)),
                    ("discard", json!(true)),
                ],
            )
            .await;
        }
        "close" => {
            settle(&editor, &tab, save, discard).await?;
            tab.m.lock().open = false;
            let _ = ui_tab(
                &editor,
                &[("action", json!("close")), ("doc", json!("pda")), ("discard", json!(true))],
            )
            .await;
        }
        "reset" => tab.m.lock().reset_state(),
        // reload keeps the Source pane's edits: it reads the game's side again
        "reload" => reload_all(&editor, &tab).await,
        "apply" => {
            // text= puts a text into the Source pane, unsaved, the way typing does; then it shows
            if let Some(text) = arg(&args, "text").filter(|t| !t.is_empty()) {
                let p = tab.m.lock().path.clone();
                let disk = editor.exec.io(move || gz_pda::read_text(Path::new(&p)).unwrap_or_default()).await;
                let mut m = tab.m.lock();
                m.text_dirty = text != disk;
                m.text = text;
            }
            read(&editor, &tab, true).await;
        }
        "save" => {
            let dirty = tab.m.lock().text_dirty;
            if dirty {
                save_text(&editor, &tab).await.map_err(invalid)?;
            }
            read(&editor, &tab, true).await;
        }
        // the caller has decided: the edits go without a question
        "revert" => {
            let p = tab.m.lock().path.clone();
            load_file(&editor, &tab, &p).await;
        }
        "page" => {
            let id = arg_or_empty(&args, "page");
            let mut m = tab.m.lock();
            m.preview.go_to_page(&id).map_err(invalid)?;
        }
        "key" => {
            // oracle: ArgString empties the default it was given, so a missing key presses the tasks key
            let key = arg_or_empty(&args, "key");
            tab.m.lock().preview.open_key(&key, &clock_now());
        }
        "aspect" => {
            let a = arg_or_empty(&args, "aspect");
            tab.m.lock().preview.aspect = Aspect::parse(&a);
        }
        "lang" => {
            // oracle: a missing lang empties it, and the reader reads rus then
            let lang = arg_or_empty(&args, "lang");
            tab.m.lock().preview.lang = lang;
            read(&editor, &tab, true).await;
        }
        "click" => click(&mut tab.m.lock(), &args)?,
        "state" => {}
        _ => return Err(invalid(format!("unknown action '{action}'"))),
    }

    // the tab's panes, its Details tab and its zoom go with any action
    let pane = arg_or_empty(&args, "pane");
    let tab_arg = arg_or_empty(&args, "tab");
    let zoom = arg_or_empty(&args, "zoom");
    {
        let mut m = tab.m.lock();
        if !pane.is_empty() {
            if !matches!(pane.as_str(), "source" | "details" | "both" | "none") {
                return Err(invalid("pane must be source, details, both or none"));
            }
            m.show_source = pane == "source" || pane == "both";
            m.show_side = pane == "details" || pane == "both";
        }
        if !tab_arg.is_empty() {
            let Some(t) = TAB_IDS.iter().position(|id| *id == tab_arg) else {
                return Err(invalid("tab must be problems, pages, vars or log"));
            };
            m.tab = t;
            m.want_tab = Some(t);
            m.show_side = true;
        }
        if !zoom.is_empty() {
            let factor = match zoom.as_str() {
                "fit" => 0.0,
                "100" => 1.0,
                z => atof(z),
            };
            if zoom != "fit" && !(ZOOM_MIN..=ZOOM_MAX).contains(&factor) {
                return Err(invalid("zoom must be fit, 100 or a factor 0.25..4"));
            }
            m.zoom = factor;
            m.pan = egui::Vec2::ZERO;
        }
    }
    // a change to what the tab draws, or a tab not drawn lately, owes the frames after this call
    let (active, frame) = shown(&editor).await;
    let drawn = tab.m.lock().drawn_frame;
    let stale = drawn.is_none_or(|d| d + 1 < frame);
    let owed = active
        && (action != "state"
            || given_path
            || !pane.is_empty()
            || !tab_arg.is_empty()
            || !zoom.is_empty()
            || stale);
    if owed {
        let _ = editor.frames.after(2).await;
    }
    Ok(answer(&editor, &tab, &assets, false, active).await)
}

/// Whether the pda tab is up, and the egui frame being drawn.
async fn shown(editor: &Arc<Editor>) -> (bool, u64) {
    gz_app::on_ui(editor, |shell, ctx| Ok((shell.active() == "pda", ctx.cumulative_frame_nr())))
        .await
        .unwrap_or((false, 0))
}

/// The answer (`AnswerPreview` and `JsonState`): the panes, the zoom and what the last frame
/// drew, then the state in canvas units.
async fn answer(
    editor: &Arc<Editor>,
    tab: &Arc<PdaTab>,
    assets: &Arc<PdaAssets>,
    pending: bool,
    active: bool,
) -> Value {
    let device = assets.device_wait(editor).await;
    let level = tab.m.lock().preview.layout.preview_level.clone();
    let map = assets.level_map_wait(editor, &level).await;
    let now = clock_now();
    let mut m = tab.m.lock();
    let wide = m.preview.aspect == Aspect::Wide;
    let geom = m.geometry(device.side(wide).noise);

    let mut out = Map::new();
    out.insert("open".into(), json!(m.open));
    out.insert("active".into(), json!(active));
    out.insert("text_dirty".into(), json!(m.text_dirty));
    if !m.disk_note.is_empty() {
        out.insert("disk_note".into(), json!(m.disk_note));
    }
    out.insert(
        "panes".into(),
        json!({
            "source": m.show_source,
            "details": m.show_side,
            "tab": TAB_IDS.get(m.tab.min(3)).copied().unwrap_or("problems"),
            "details_width": whole(m.details_w),
            "source_height": whole(m.source_h),
        }),
    );
    out.insert("zoom".into(), if m.zoom > 0.0 { g(m.zoom) } else { json!("fit") });
    let scale = m.geom.map_or(1.0, |gm| gm.ky * m.ppp);
    out.insert("scale".into(), fixed(scale, 3));
    out.insert("device_gap".into(), fixed(m.device_gap, 1));
    out.insert("read_line".into(), json!(if m.read_ok { 0 } else { m.read_line }));
    let edges: Vec<&str> = ["top", "bottom", "left", "right"]
        .iter()
        .enumerate()
        .filter(|(i, _)| m.screen_edges & (1 << i) != 0)
        .map(|(_, n)| *n)
        .collect();
    out.insert("screen_edges".into(), json!(edges));
    out.insert("pending".into(), json!(pending));

    // the state, in CANVAS units: the coordinates an agent writes into the file
    let mut st = Map::new();
    st.insert("path".into(), json!(m.rel.replace('\\', "/")));
    st.insert("page".into(), json!(m.preview.page));
    st.insert("stock".into(), json!(m.preview.stock));
    st.insert("closed".into(), json!(m.preview.closed));
    st.insert("aspect".into(), json!(m.preview.aspect.label()));
    st.insert("lang".into(), json!(m.preview.lang));
    st.insert("read".into(), json!(m.read_ok));
    if !m.read_ok {
        let e = if m.read_line > 0 {
            format!("line {}: {}", m.read_line, m.read_err)
        } else {
            m.read_err.clone()
        };
        st.insert("read_error".into(), json!(e));
    }
    st.insert("errors".into(), json!(m.preview.layout.errors));
    st.insert("warnings".into(), json!(m.preview.layout.warnings));
    let (boxes, rects) = item_rects(&m.preview, &geom);
    // the preview map's world: where preview.actor and preview.markers land (x right, z up)
    if let Some(lm) = &map {
        let mut mo = Map::new();
        mo.insert("level".into(), json!(level));
        mo.insert("bound".into(), json!(lm.bound.iter().map(|v| g(*v)).collect::<Vec<_>>()));
        mo.insert("zoom".into(), g(m.preview.map_zoom));
        // what the slot shows, in canvas units - the game's describe() lists its spots the same way
        let layout = m.preview.layout.clone();
        for (i, &(slot, shown)) in rects.iter().enumerate() {
            if !shown || layout.items.get(i).is_none_or(|it| it.kind != "map") {
                continue;
            }
            let Some(v) = map_view(&mut m, lm, slot) else { continue };
            let root = geom.root();
            let (cx, cy) = (geom.cx(layout.canvas_w), geom.cy(layout.canvas_h));
            let mut spots = Vec::new();
            let mut spot = |what: &str, hint: &str, p: egui::Pos2| {
                if !slot.contains(p) {
                    return;
                }
                let mut s = Map::new();
                s.insert("what".into(), json!(what));
                s.insert("x".into(), whole((p.x - root.x) / cx));
                s.insert("y".into(), whole((p.y - root.y) / cy));
                if !hint.is_empty() {
                    s.insert("hint".into(), json!(hint));
                }
                spots.push(Value::Object(s));
            };
            spot("player", "", layout.actor.map_or(v.mid, |(x, z)| v.w2p(x, z)));
            for mk in &layout.markers {
                spot("marker", &mk.hint, v.w2p(mk.x, mk.z));
            }
            mo.insert("spots".into(), Value::Array(spots));
            break;
        }
        st.insert("map".into(), Value::Object(mo));
    }
    let vars: Map<String, Value> = m.preview.vars_view().into_iter().map(|(k, v)| (k, json!(v))).collect();
    st.insert("vars".into(), Value::Object(vars));
    st.insert("log".into(), json!(m.preview.log));
    let visible: Vec<Value> = m
        .preview
        .visible(&boxes, &now)
        .into_iter()
        .map(|it| {
            let mut o = Map::new();
            o.insert("n".into(), json!(it.n));
            o.insert("kind".into(), json!(it.kind));
            if !it.id.is_empty() {
                o.insert("id".into(), json!(it.id));
            }
            if let Some((row, sel)) = it.row {
                o.insert("row".into(), json!(row));
                o.insert("selected".into(), json!(sel));
            }
            o.insert("x".into(), whole(it.x));
            o.insert("y".into(), whole(it.y));
            o.insert("w".into(), whole(it.w));
            o.insert("h".into(), whole(it.h));
            if let Some(t) = it.text {
                o.insert("text".into(), json!(t));
            }
            if !it.action.is_empty() {
                o.insert("action".into(), json!(it.action));
            }
            if let Some(sel) = it.selected {
                o.insert("selected".into(), json!(sel));
            }
            Value::Object(o)
        })
        .collect();
    st.insert("visible".into(), Value::Array(visible));
    out.insert("state".into(), Value::Object(st));
    Value::Object(out)
}
