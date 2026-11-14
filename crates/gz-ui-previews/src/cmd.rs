//! The commands of the previews: `preview.model`, `preview.texture`, `library.view`, `library.set`,
//! `library.play`, `library.import`, `library.rebuild` and `shot.asset`.
//!
//! A command that changes what a tab draws applies itself, then waits for the frames that show
//! it and reads the tab again: the answer is the frame it asked for.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use gz_app::on_ui;
use gz_editor::{Args, CmdError, Editor, Origin, Registry, Reply, Spec};
use gz_render::{Instance, InstanceFlags, Renderer, View, ViewFlags, build_packet};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::library::lanim::LanimDoc;
use crate::library::sounds::SoundLibDoc;
use crate::library::textures::TexLibDoc;
use crate::library::{KINDS, LibraryDoc};
use crate::model::{ModelDoc, ModelPreload, Orbit};
use crate::model_load::{ModelSource, load as load_model};
use crate::paths::engine_name;
use crate::pending::Pending;
use crate::texture::{TextureDoc, TexturePreload, parse_channel};
use crate::texture_load::{TexSource, UNREADABLE, UNREADABLE_REPLY, load as load_texture};

const LOAD_WAIT: Duration = Duration::from_secs(60);

fn invalid(text: impl Into<String>) -> CmdError {
    CmdError::invalid(text)
}

/// The frames an answer owes: drawn, and read again. Without a window there are none.
async fn owe_frames(editor: &Editor, n: u64) {
    if editor.frames.has_window() {
        let _ = editor.frames.after(n).await;
    }
}

/// Opens a tab through the shell's own opener (`ui.tab open`), so a person and an agent share
/// one path; an error reply becomes this command's error.
async fn open_tab(
    editor: &Arc<Editor>,
    kind: &str,
    path: Option<&str>,
    source: Option<&str>,
) -> Result<(), CmdError> {
    let mut args = Args::new();
    args.insert("action".into(), json!("open"));
    args.insert("kind".into(), json!(kind));
    if let Some(p) = path {
        args.insert("path".into(), json!(p));
    }
    if let Some(s) = source {
        args.insert("source".into(), json!(s));
    }
    let reply = editor.call("ui.tab", args, Origin::Mcp).await;
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let text = reply.get("error").and_then(Value::as_str).unwrap_or("the tab did not open").to_owned();
    let mut e = CmdError::invalid(text);
    if let Value::Object(mut data) = reply {
        data.remove("ok");
        data.remove("error");
        if !data.is_empty() {
            e = e.with_data(Value::Object(data));
        }
    }
    Err(e)
}

fn doc<'a, T: 'static>(shell: &'a mut gz_app::ShellState, id: &str) -> Option<&'a mut T> {
    shell.document_mut(id).and_then(|d| d.as_any_mut()).and_then(|a| a.downcast_mut::<T>())
}

fn merge(head: Map<String, Value>, tail: Map<String, Value>) -> Value {
    let mut out = head;
    for (k, v) in tail {
        out.insert(k, v);
    }
    Value::Object(out)
}

// ---- preview.model -----------------------------------------------------------------------------

/// How the model preview lights the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Lighting {
    /// A headlight along the view and a key from above, over an ambient.
    Studio,
    /// The textures as they are.
    Unlit,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ModelParams {
    /// A visual (`dynamics\box\box_1a`) or a library object (`props\box_wood_01`) to show; left
    /// out: the tab open now.
    #[serde(default)]
    name: Option<String>,
    /// Where `name` is looked for: `library` (the project, the library, then the game - the
    /// default), `project`, or `game` (the linked game's visuals only).
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    lighting: Option<Lighting>,
    /// Draws the triangles' edges only.
    #[serde(default)]
    wireframe: Option<bool>,
    /// Draws the skeleton over the model.
    #[serde(default)]
    bones: Option<bool>,
    /// Shows the Details panel.
    #[serde(default)]
    details: Option<bool>,
    /// Frames the model, keeping the angle.
    #[serde(default)]
    frame: bool,
    /// Back to the three-quarter view, framed.
    #[serde(default)]
    reset: bool,
}

/// What the model tab answers: `tab`, `active` and its state.
async fn model_answer(editor: &Arc<Editor>) -> Reply {
    let (active, state) = on_ui(editor, |shell, _| {
        let active = shell.active() == "model";
        let state = doc::<ModelDoc>(shell, "model").map(|d| d.state_json());
        Ok((active, state))
    })
    .await?;
    let mut head = Map::new();
    head.insert("tab".into(), json!("model"));
    head.insert("active".into(), json!(active));
    let tail = state.unwrap_or_else(|| Map::from_iter([("open".to_owned(), json!(false))]));
    Ok(merge(head, tail))
}

async fn preview_model(editor: Arc<Editor>, p: ModelParams, _origin: Origin) -> Reply {
    let name = engine_name(p.name.as_deref().unwrap_or_default());
    let Some(source) = ModelSource::parse(p.source.as_deref().unwrap_or_default()) else {
        return Err(invalid("source takes library, project or game"));
    };
    let lighting = p.lighting.map(|l| usize::from(l == Lighting::Unlit));
    let (wireframe, bones, details) = (p.wireframe, p.bones, p.details);
    let (frame, reset) = (p.frame, p.reset);

    if !name.is_empty() {
        // already showing this one: the tab comes forward
        let (n, s) = (name.clone(), source);
        let showing = on_ui(&editor, move |shell, ctx| {
            let same = doc::<ModelDoc>(shell, "model").is_some_and(|d| d.name() == n && d.source() == s);
            if same {
                shell.activate(ctx, "model");
            }
            Ok(same)
        })
        .await?;
        if !showing {
            let pending = Pending::spawn(&editor.exec, load_model(editor.clone(), name.clone(), source));
            let loaded = pending.wait(LOAD_WAIT).await;
            if source.is_game()
                && loaded.as_ref().is_some_and(|d| d.error == "not found in the linked game install")
            {
                return Err(invalid("not found in the linked game install"));
            }
            editor.services.get_or_init(ModelPreload::default).put(&name, source, pending);
            open_tab(&editor, "model", Some(&name), Some(source.arg())).await?;
        }
    } else {
        let open = on_ui(&editor, |shell, ctx| Ok(shell.activate(ctx, "model"))).await?;
        if !open {
            return model_answer(&editor).await;
        }
    }

    let asked = !name.is_empty()
        || lighting.is_some()
        || wireframe.is_some()
        || bones.is_some()
        || details.is_some()
        || frame
        || reset;
    let (active, drawn) = on_ui(&editor, move |shell, ctx| {
        let frame_nr = ctx.cumulative_frame_nr();
        let active = shell.active() == "model";
        let Some(d) = doc::<ModelDoc>(shell, "model") else { return Err(invalid("no model is open")) };
        d.apply_view(lighting, wireframe, bones, details, frame, reset);
        Ok((active, d.drawn_lately(frame_nr)))
    })
    .await?;
    if active && (asked || !drawn) {
        // the tab draws once and takes the model; everything it draws is loaded; then a packet
        // made with all of it is drawn, and the frames after that show the picture
        owe_frames(&editor, 2).await;
        let ready = on_ui(&editor, |shell, ctx| {
            let Some(d) = doc::<ModelDoc>(shell, "model") else { return Ok(None) };
            Ok(d.renderer().zip(d.draw_list(ctx)))
        })
        .await?;
        if let Some((renderer, list)) = ready {
            let r = renderer.clone();
            editor.exec.io(move || r.assets().prepare(&list, Duration::from_secs(10))).await;
            let before =
                on_ui(&editor, |shell, _| Ok(doc::<ModelDoc>(shell, "model").map_or(0, |d| d.redraw())))
                    .await?;
            let mut serial = before;
            for _ in 0..30 {
                owe_frames(&editor, 1).await;
                serial = on_ui(&editor, |shell, _| {
                    Ok(doc::<ModelDoc>(shell, "model").map_or(0, |d| d.submitted()))
                })
                .await?;
                if serial > before || !editor.frames.has_window() {
                    break;
                }
            }
            if serial > before {
                editor.exec.io(move || renderer.wait_frame(serial, Duration::from_secs(10)).is_ok()).await;
            }
        }
        owe_frames(&editor, 3).await;
    }
    model_answer(&editor).await
}

// ---- preview.texture ---------------------------------------------------------------------------

/// What `preview.texture` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum TextureAction {
    /// Answers the tab's state; the view settings given still apply.
    #[default]
    State,
    /// Opens `path` from `source`.
    Open,
    /// Closes the tab.
    Close,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TextureParams {
    #[serde(default)]
    action: TextureAction,
    /// open: the texture - a project path (`gamedata\textures\ui\x.dds`), a library name
    /// without its extension (`ui\x`) or a game path (`textures\ui\x.dds`).
    #[serde(default)]
    path: Option<String>,
    /// open: `project`, `library` or `game`.
    #[serde(default)]
    source: Option<String>,
    /// `fit`, or a factor (`1` is one texel to one pixel).
    #[serde(default)]
    zoom: Option<String>,
    /// rgba, r, g, b or a.
    #[serde(default)]
    channel: Option<String>,
    /// The mip level shown, from 0.
    #[serde(default)]
    mip: Option<u32>,
    /// Shows the Details panel.
    #[serde(default)]
    details: Option<bool>,
}

async fn texture_answer(editor: &Arc<Editor>) -> Reply {
    let (active, state) = on_ui(editor, |shell, _| {
        let active = shell.active() == "texture";
        let state = doc::<TextureDoc>(shell, "texture").map(|d| d.state_json());
        Ok((active, state))
    })
    .await?;
    let mut head = Map::new();
    head.insert("tab".into(), json!("texture"));
    head.insert("active".into(), json!(active));
    let tail = state.unwrap_or_else(|| Map::from_iter([("open".to_owned(), json!(false))]));
    Ok(merge(head, tail))
}

async fn texture_preview(editor: Arc<Editor>, p: TextureParams, _origin: Origin) -> Reply {
    let mut asked = false;
    match p.action {
        TextureAction::Open => {
            let path = p.path.as_deref().unwrap_or_default().replace('/', "\\");
            let source = TexSource::parse(p.source.as_deref().unwrap_or_default());
            let Some(source) = source.filter(|_| !path.is_empty()) else {
                return Err(invalid("open takes path= and source=project|library|game"));
            };
            let (p, s) = (path.clone(), source);
            let showing = on_ui(&editor, move |shell, ctx| {
                let same = doc::<TextureDoc>(shell, "texture").is_some_and(|d| {
                    d.name() == p && d.source() == s && d.data().is_some_and(|t| t.texture.is_some())
                });
                if same {
                    shell.activate(ctx, "texture");
                }
                Ok(same)
            })
            .await?;
            if !showing {
                let pending =
                    Pending::spawn(&editor.exec, load_texture(editor.clone(), path.clone(), source));
                let loaded = pending.wait(LOAD_WAIT).await;
                editor.services.get_or_init(TexturePreload::default).put(&path, source, pending);
                open_tab(&editor, "texture", Some(&path), Some(source.arg())).await?;
                if let Some(t) = loaded
                    && !t.error.is_empty()
                {
                    return Err(invalid(if t.error == UNREADABLE {
                        UNREADABLE_REPLY.to_owned()
                    } else {
                        t.error.clone()
                    }));
                }
            }
            asked = true;
        }
        TextureAction::Close => {
            let open = on_ui(&editor, |shell, _| Ok(shell.document_mut("texture").is_some())).await?;
            if open {
                let mut a = Args::new();
                a.insert("action".into(), json!("close"));
                a.insert("doc".into(), json!("texture"));
                a.insert("discard".into(), json!(true));
                let _ = editor.call("ui.tab", a, Origin::Mcp).await;
            }
            return texture_answer(&editor).await;
        }
        TextureAction::State => {}
    }
    let open = on_ui(&editor, |shell, _| Ok(shell.document_mut("texture").is_some())).await?;
    if !open {
        return texture_answer(&editor).await;
    }
    let zoom_text = p.zoom.clone().unwrap_or_default();
    let zoom = match zoom_text.as_str() {
        "" => None,
        "fit" => Some(0.0),
        z => Some(z.trim().parse::<f32>().unwrap_or(0.0)),
    };
    let channel_text = p.channel.as_deref().unwrap_or_default();
    let channel = if channel_text.is_empty() {
        None
    } else {
        parse_channel(channel_text).map(Some).unwrap_or(Some(usize::MAX))
    };
    if channel == Some(usize::MAX) || (zoom_text != "fit" && zoom.is_some_and(|z| z <= 0.0 || !z.is_finite()))
    {
        return Err(invalid("zoom is fit or a factor, channel rgba, r, g, b or a"));
    }
    let (mip, details) = (p.mip, p.details);
    asked = asked || zoom.is_some() || channel.is_some() || mip.is_some() || details.is_some();
    let (active, drawn) = on_ui(&editor, move |shell, ctx| {
        let frame_nr = ctx.cumulative_frame_nr();
        let active = shell.active() == "texture";
        let Some(d) = doc::<TextureDoc>(shell, "texture") else { return Err(invalid("no texture is open")) };
        d.apply_view(zoom, mip, channel, details).map_err(invalid)?;
        Ok((active, d.drawn_lately(frame_nr)))
    })
    .await?;
    if active && (asked || !drawn) {
        owe_frames(&editor, 3).await;
    }
    texture_answer(&editor).await
}

// ---- the libraries -----------------------------------------------------------------------------

/// The three libraries, as the tabs and the commands name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum LibKind {
    /// `gamedata\lanims.xr`.
    LightAnimations,
    /// The `.tga` sources of `rawdata\textures` with their descriptions.
    TextureLibrary,
    /// The `.wav` sources of `rawdata\sounds` with their descriptions.
    SoundLibrary,
}

impl LibKind {
    fn word(self) -> &'static str {
        match self {
            LibKind::LightAnimations => KINDS[0],
            LibKind::TextureLibrary => KINDS[1],
            LibKind::SoundLibrary => KINDS[2],
        }
    }
}

/// What `library.view` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum LibraryAction {
    /// Answers the tab's state (the tab must be open).
    #[default]
    State,
    /// Opens the tab and brings it up.
    Open,
    /// Selects `item` in the tab's list.
    Select,
    /// Texture library: looks in the import folder and turns the tab into New Textures.
    Check,
}

/// The key of a light animation the timeline moves to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum KeyStep {
    Previous,
    Next,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LibraryParams {
    kind: LibKind,
    #[serde(default)]
    action: LibraryAction,
    /// select: the entry by its name (or a part of it).
    #[serde(default)]
    item: Option<String>,
    /// select (light animations): the frame shown.
    #[serde(default)]
    frame: Option<i64>,
    /// The list's search; empty clears it.
    #[serde(default)]
    query: Option<String>,
    /// Light animations: show the colours' alpha over a checkerboard.
    #[serde(default)]
    alpha: Option<bool>,
    /// Sound library: play every sound as it is picked in the list.
    #[serde(default)]
    auto_play: Option<bool>,
    /// Texture library: the kinds of texture the list shows - images, cube_maps, bump_maps,
    /// normal_maps, terrain. Empty shows none.
    #[serde(default)]
    types: Option<Vec<String>>,
    /// Light animations: the timeline goes to the previous or the next key of the selected one.
    #[serde(default)]
    key: Option<KeyStep>,
    /// Files (separated by |) dropped on the tab at the window point `x`, `y`, the way a drop from
    /// Explorer lands. Pictures dropped on the Texture Library are imported.
    #[serde(default)]
    simulate_drop: Option<String>,
    /// The window point of `simulate_drop`, from the left, in pixels.
    #[serde(default)]
    x: Option<f32>,
    /// The window point of `simulate_drop`, from the top, in pixels.
    #[serde(default)]
    y: Option<f32>,
}

fn doc_of_kind(shell: &mut gz_app::ShellState, kind: LibKind) -> Option<&mut dyn LibraryDoc> {
    let k = kind.word();
    match kind {
        LibKind::LightAnimations => doc::<LanimDoc>(shell, k).map(|d| d as &mut dyn LibraryDoc),
        LibKind::TextureLibrary => doc::<TexLibDoc>(shell, k).map(|d| d as &mut dyn LibraryDoc),
        LibKind::SoundLibrary => doc::<SoundLibDoc>(shell, k).map(|d| d as &mut dyn LibraryDoc),
    }
}

fn not_open(kind: LibKind) -> CmdError {
    invalid(format!("the {} tab is not open: library.view action=open opens it", kind.word()))
}

/// Asks the tab of `kind` for what it answers: `action` and `args` applied, then its fields. The
/// tab must be open. Also whether it is the active tab and what a selection must wait for.
async fn run_tab(
    editor: &Arc<Editor>,
    kind: LibKind,
    action: &'static str,
    args: Args,
) -> Result<(bool, Map<String, Value>, Option<crate::library::Settle>), CmdError> {
    on_ui(editor, move |shell, _| {
        let active = shell.active() == kind.word();
        let Some(d) = doc_of_kind(shell, kind) else { return Err(not_open(kind)) };
        let body = d.mcp(action, &args).map_err(invalid)?;
        let settle = if action == "select" { d.settle() } else { None };
        Ok((active, body, settle))
    })
    .await
}

/// Whether the tab of `kind` is open.
async fn tab_open(editor: &Arc<Editor>, kind: LibKind) -> Result<bool, CmdError> {
    on_ui(editor, move |shell, _| Ok(shell.document_mut(kind.word()).is_some())).await
}

/// The entry the tab of `kind` has selected, when it is open and has one.
async fn selected_of(editor: &Arc<Editor>, kind: LibKind) -> Option<String> {
    on_ui(editor, move |shell, _| {
        Ok(match kind {
            LibKind::LightAnimations => doc::<LanimDoc>(shell, kind.word()).map(|d| d.selected.clone()),
            LibKind::TextureLibrary => doc::<TexLibDoc>(shell, kind.word()).map(|d| d.selected.clone()),
            LibKind::SoundLibrary => doc::<SoundLibDoc>(shell, kind.word()).map(|d| d.selected.clone()),
        })
    })
    .await
    .ok()
    .flatten()
    .filter(|s| !s.is_empty())
}

/// What a library is waiting for before a command may answer about it.
async fn list_ready(editor: &Arc<Editor>, kind: LibKind) -> Result<(), CmdError> {
    match kind {
        LibKind::LightAnimations => {
            if let Some(store) =
                on_ui(editor, |shell, _| Ok(doc::<LanimDoc>(shell, "light_animations").map(|d| d.store())))
                    .await?
            {
                LanimDoc::loaded(&store).await;
            }
        }
        LibKind::TextureLibrary => {
            let import = on_ui(editor, |shell, _| {
                Ok(doc::<TexLibDoc>(shell, "texture_library").and_then(|d| d.import_pending()))
            })
            .await?;
            TexLibDoc::items_now(editor, import).await;
        }
        LibKind::SoundLibrary => {
            SoundLibDoc::items_now(editor).await;
        }
    }
    Ok(())
}

async fn library(editor: Arc<Editor>, p: LibraryParams, _origin: Origin) -> Reply {
    let kind = p.kind;
    let mut action = match p.action {
        LibraryAction::State => "state",
        LibraryAction::Open => "open",
        LibraryAction::Select => "select",
        LibraryAction::Check => "check",
    };
    // what the tabs read: their own words for the same things
    let mut args = Args::new();
    for (key, value) in [
        ("item", p.item.as_ref().map(|v| json!(v))),
        ("frame", p.frame.map(|v| json!(v))),
        ("search", p.query.as_ref().map(|v| json!(v))),
        ("alpha", p.alpha.map(|v| json!(v))),
        ("auto_play", p.auto_play.map(|v| json!(v))),
        ("types", p.types.as_ref().map(|v| json!(v))),
        ("key", p.key.map(|k| json!(if k == KeyStep::Previous { "previous" } else { "next" }))),
    ] {
        if let Some(v) = value {
            args.insert(key.into(), v);
        }
    }
    // open brings the tab up; state and select never switch tabs
    if action == "open" || action == "check" {
        let k = kind.word();
        let there =
            on_ui(&editor, move |shell, ctx| Ok(shell.document_mut(k).is_some() && shell.activate(ctx, k)))
                .await?;
        if !there {
            open_tab(&editor, kind.word(), None, None).await?;
        }
    }
    if !tab_open(&editor, kind).await? {
        return Err(not_open(kind));
    }
    if action == "check" {
        if kind != LibKind::TextureLibrary {
            return Err(invalid("check looks in the import folder: it is the texture library's"));
        }
        run_tab(&editor, kind, "check", Args::new()).await?;
        action = "state";
    }
    if let Some(list) = &p.simulate_drop {
        let files: Vec<std::path::PathBuf> = list
            .split('|')
            .map(|f| f.trim_matches([' ', '\t', '\r', '\n']))
            .filter(|f| !f.is_empty())
            .map(std::path::PathBuf::from)
            .collect();
        let (x, y) = (p.x.unwrap_or(0.0), p.y.unwrap_or(0.0));
        on_ui(&editor, move |_, ctx| {
            let ppp = ctx.pixels_per_point();
            gz_ui::drop::simulate_files(ctx, files, egui::pos2(x / ppp, y / ppp));
            Ok(())
        })
        .await?;
        owe_frames(&editor, 3).await;
        // what the drop started ends before the answer: the import it ran
        let stores = crate::library::stores::LibraryStores::of(&editor);
        let begun = tokio::time::Instant::now() + Duration::from_secs(3);
        while !crate::library::stores::is_set(&stores.tex_saving) && tokio::time::Instant::now() < begun {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let ended = tokio::time::Instant::now() + LOAD_WAIT;
        while crate::library::stores::is_set(&stores.tex_saving) && tokio::time::Instant::now() < ended {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        owe_frames(&editor, 2).await;
    }
    // what the tab answers about is its list as read
    list_ready(&editor, kind).await?;
    let (mut active, mut body, settle) = run_tab(&editor, kind, action, args).await?;
    // a selection answers once what it shows is read: the picture, the game sound
    if let Some(s) = settle {
        s.await;
        (active, body, _) = run_tab(&editor, kind, "state", Args::new()).await?;
    }
    let mut head = Map::new();
    head.insert("tab".into(), json!(kind.word()));
    head.insert("active".into(), json!(active));
    if active {
        owe_frames(&editor, 2).await;
    }
    Ok(merge(head, std::mem::take(&mut body)))
}

fn yes() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetParams {
    kind: LibKind,
    /// The entry by its exact name (any case): a texture `wpn\wpn_ak74`, a sound, a light
    /// animation. Left out: the one the tab has selected.
    #[serde(default)]
    item: Option<String>,
    /// What to change, by key. Texture: type, format, mips, mip_filter, bump_mode, bump_texture,
    /// bump_normal_map, bump_height, detail_texture, detail_scale, detail_diffuse, detail_bump,
    /// material, material_weight, dither, dither_each_mip, implicit_lighted, fade_color_on,
    /// fade_alpha_on, fade_delay, fade_amount, fade_color, border_color_on, border_alpha_on,
    /// border_color. Sound: quality, min_dist, max_dist, max_ai_dist, base_volume, game_type.
    /// Light animation: fps, frames, keys ({"<frame>": "#RRGGBB" or null}), move_key ({from, to}).
    /// A word is the one the editor shows (`DXT5`, `Cube Map`), any case. An unknown key is
    /// refused with the nearest real one.
    set: Map<String, Value>,
    /// Write the files too, as the tab's Save does: the description and the game texture or
    /// sound built from it, or lanims.xr. False leaves the change as an edit of the tab
    /// (undoable, unsaved) for `doc.save`.
    #[serde(default = "yes")]
    save: bool,
}

/// What a texture's or a sound's edit is, by the entry's description as the tab shows it.
async fn set_entry(editor: &Arc<Editor>, p: SetParams, origin: Origin) -> Reply {
    let item = match p.item.filter(|i| !i.is_empty()) {
        Some(i) => i,
        None => selected_of(editor, p.kind)
            .await
            .ok_or_else(|| invalid("item names the entry (nothing is selected in a tab)"))?,
    };
    let mut head = Map::new();
    head.insert("kind".into(), json!(p.kind.word()));
    let stores = crate::library::stores::LibraryStores::of(editor);
    match p.kind {
        LibKind::TextureLibrary => {
            let items = TexLibDoc::items_now(editor, None).await;
            let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
            let name = crate::library::keys::exact("texture", &names, &item)
                .map_err(CmdError::not_found)?
                .to_owned();
            let found =
                items.iter().find(|i| i.name == name).cloned().ok_or_else(|| CmdError::not_found(&name))?;
            let pending = stores.tex.snapshot().edits.get(&name).cloned();
            let base = pending.clone().unwrap_or(found.params);
            let mut params = base.clone();
            let changed = crate::library::keys::apply_texture(&mut params, &p.set).map_err(invalid)?;
            if params != base {
                stores
                    .tex
                    .send(
                        crate::library::textures::TexCmd::Set {
                            name: name.clone(),
                            params,
                            gesture: "set".into(),
                        },
                        origin,
                    )
                    .await?;
            }
            let write = p.save && (pending.is_some() || !changed.is_empty());
            if write {
                let id = crate::library::textures::start_texture_save(editor, None, Some(vec![name.clone()]))
                    .map_err(CmdError::busy)?;
                crate::library::textures::job_answer(editor, id).await?;
            }
            head.insert("item".into(), json!(name));
            head.insert("changed".into(), json!(changed));
            head.insert("saved".into(), json!(write));
            head.insert("dirty".into(), json!(stores.tex.is_dirty()));
        }
        LibKind::SoundLibrary => {
            let items = SoundLibDoc::items_now(editor).await;
            let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
            let name =
                crate::library::keys::exact("sound", &names, &item).map_err(CmdError::not_found)?.to_owned();
            let found =
                items.iter().find(|i| i.name == name).cloned().ok_or_else(|| CmdError::not_found(&name))?;
            let pending = stores.sound.snapshot().edits.get(&name).copied();
            let base = pending.unwrap_or(found.thm);
            let mut thm = base;
            let changed = crate::library::keys::apply_sound(&mut thm, &p.set).map_err(invalid)?;
            if thm != base {
                stores
                    .sound
                    .send(
                        crate::library::sounds::SoundCmd::Set {
                            name: name.clone(),
                            thm,
                            gesture: "set".into(),
                        },
                        origin,
                    )
                    .await?;
            }
            let write = p.save && (pending.is_some() || !changed.is_empty());
            if write {
                let id = crate::library::sounds::start_sound_save(editor, Some(vec![name.clone()]))
                    .map_err(CmdError::busy)?;
                crate::library::textures::job_answer(editor, id).await?;
            }
            head.insert("item".into(), json!(name));
            head.insert("changed".into(), json!(changed));
            head.insert("saved".into(), json!(write));
            head.insert("dirty".into(), json!(stores.sound.is_dirty()));
        }
        LibKind::LightAnimations => {
            let done = crate::library::lanim::set_lanim(editor, &item, &p.set).await?;
            let write = p.save && !done.changed.is_empty();
            if write {
                crate::library::lanim::save_lanims(editor).await.map_err(CmdError::io)?;
            }
            if let Some(frame) = done.follow {
                let _ = on_ui(editor, move |shell, _| {
                    if let Some(d) = doc::<LanimDoc>(shell, "light_animations") {
                        d.set_pointer(frame);
                    }
                    Ok(())
                })
                .await;
            }
            head.insert("item".into(), json!(item));
            head.insert("changed".into(), json!(done.changed));
            head.insert("saved".into(), json!(write));
            head.insert("dirty".into(), json!(stores.lanim.is_dirty()));
        }
    }
    owe_frames(editor, 1).await;
    Ok(Value::Object(head))
}

async fn library_set(editor: Arc<Editor>, p: SetParams, origin: Origin) -> Reply {
    set_entry(&editor, p, origin).await
}

/// What `library.play` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PlayAction {
    /// Plays the selected entry.
    #[default]
    Play,
    /// Stops what plays.
    Stop,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PlayParams {
    /// sound_library: the game sound; light_animations: the animation in real time.
    kind: LibKind,
    #[serde(default)]
    action: PlayAction,
    /// play: the entry to pick first; left out: the one selected.
    #[serde(default)]
    item: Option<String>,
}

async fn library_play(editor: Arc<Editor>, p: PlayParams, _origin: Origin) -> Reply {
    if p.kind == LibKind::TextureLibrary {
        return Err(invalid("a texture plays nothing: kind is sound_library or light_animations"));
    }
    let kind = p.kind;
    if !tab_open(&editor, kind).await? {
        return Err(not_open(kind));
    }
    list_ready(&editor, kind).await?;
    let play = p.action == PlayAction::Play;
    if play && let Some(item) = &p.item {
        let mut a = Args::new();
        a.insert("item".into(), json!(item));
        let (_, _, settle) = run_tab(&editor, kind, "select", a).await?;
        if let Some(s) = settle {
            s.await;
        }
    }
    let answered = on_ui(&editor, move |shell, ctx| {
        let said = match kind {
            LibKind::SoundLibrary => doc::<SoundLibDoc>(shell, kind.word()).map(|d| d.play(play)),
            _ => {
                let now = ctx.input(|i| i.time);
                doc::<LanimDoc>(shell, kind.word()).map(|d| d.play(play, now))
            }
        };
        match said {
            Some(r) => r.map_err(invalid),
            None => Err(not_open(kind)),
        }
    })
    .await;
    answered?;
    owe_frames(&editor, 2).await;
    let (active, body, _) = run_tab(&editor, kind, "state", Args::new()).await?;
    let mut head = Map::new();
    head.insert("tab".into(), json!(kind.word()));
    head.insert("active".into(), json!(active));
    Ok(merge(head, body))
}

pub(crate) fn yes_wait() -> bool {
    true
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ImportParams {
    /// Picture files anywhere on the disk (tga, bmp, dds, png, jpg): copied into the import folder
    /// and taken into the library. Left out: the pictures the import folder already holds.
    #[serde(default)]
    from: Option<Vec<String>>,
    /// Description keys (`library.set`'s texture keys) applied to every picture imported.
    #[serde(default)]
    set: Option<Map<String, Value>>,
    /// Replace what is there: a staged picture of the same name and a texture of the library
    /// that an imported picture has the name of. Without it the call refuses and names them.
    #[serde(default)]
    overwrite: bool,
    /// Answer when the job has ended; false answers the job at once.
    #[serde(default = "yes_wait")]
    wait: bool,
}

async fn library_import(editor: Arc<Editor>, p: ImportParams, _origin: Origin) -> Reply {
    let request = crate::library::textures::ImportRequest {
        from: p.from.unwrap_or_default().into_iter().map(std::path::PathBuf::from).collect(),
        set: p.set.unwrap_or_default(),
        overwrite: p.overwrite,
        wait: p.wait,
    };
    crate::library::textures::import_pictures(&editor, request).await
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RebuildParams {
    /// texture_library: the game texture; sound_library: the game sound.
    kind: LibKind,
    /// The entry by its exact name; left out: the one the tab has selected.
    #[serde(default)]
    item: Option<String>,
    /// texture_library: a cube map's smaller copy, `<name>#small.dds` of 32 x 32 a face.
    #[serde(default)]
    small: bool,
    /// Answer when the job has ended; false answers the job at once.
    #[serde(default = "yes_wait")]
    wait: bool,
}

async fn library_rebuild(editor: Arc<Editor>, p: RebuildParams, _origin: Origin) -> Reply {
    let item = match p.item.filter(|i| !i.is_empty()) {
        Some(i) => i,
        None => selected_of(&editor, p.kind)
            .await
            .ok_or_else(|| invalid("item names the entry (nothing is selected in a tab)"))?,
    };
    let stores = crate::library::stores::LibraryStores::of(&editor);
    let id = match p.kind {
        LibKind::TextureLibrary => {
            let items = TexLibDoc::items_now(&editor, None).await;
            let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
            let name = crate::library::keys::exact("texture", &names, &item)
                .map_err(CmdError::not_found)?
                .to_owned();
            if p.small {
                crate::libtools::smaller_cube_map(&editor, &name).map_err(CmdError::busy)?
            } else {
                let pending = stores.tex.snapshot().edits.get(&name).cloned();
                crate::library::textures::start_texture_rebuild(&editor, &name, pending)
                    .map_err(CmdError::busy)?
            }
        }
        LibKind::SoundLibrary => {
            if p.small {
                return Err(invalid("small is a cube map's: kind=texture_library"));
            }
            let items = SoundLibDoc::items_now(&editor).await;
            let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
            let name =
                crate::library::keys::exact("sound", &names, &item).map_err(CmdError::not_found)?.to_owned();
            let found =
                items.iter().find(|i| i.name == name).cloned().ok_or_else(|| CmdError::not_found(&name))?;
            let thm = stores.sound.snapshot().edits.get(&name).copied().unwrap_or(found.thm);
            crate::library::sounds::start_sound_rebuild(&editor, &name, thm).map_err(CmdError::busy)?
        }
        LibKind::LightAnimations => return Err(invalid("a light animation has no game file to build")),
    };
    if !p.wait {
        return Ok(json!({ "job": id.0 }));
    }
    crate::library::textures::job_answer(&editor, id).await
}

// ---- shot.asset --------------------------------------------------------------------------------

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AssetShotParams {
    /// The asset: a library object (`props\box_wood_01`), a group, a texture name (`ui\x`) or a
    /// visual (`dynamics\box\box_1a`).
    name: String,
    /// objects (the default), groups, textures, raw_textures, or visuals - which the name is.
    #[serde(default)]
    kind: Option<String>,
    /// visuals: `game` takes the linked game's visual only.
    #[serde(default)]
    source: Option<String>,
}

/// The renderer the thumbnails are drawn with, made on first use from the window's device; one
/// picture at a time.
struct ThumbRenderer {
    renderer: tokio::sync::Mutex<Option<Result<Arc<Renderer>, String>>>,
}

async fn thumb_renderer(editor: &Arc<Editor>) -> Result<Arc<Renderer>, String> {
    let svc = editor.services.get_or_init(|| ThumbRenderer { renderer: tokio::sync::Mutex::new(None) });
    let mut slot = svc.renderer.lock().await;
    if slot.is_none() {
        let Some(gpu) = editor.services.get::<gz_app::WindowLink>().and_then(|l| l.gpu()) else {
            return Err("the editor runs without a graphics device".to_owned());
        };
        let (device, queue, exec) = (gpu.device.clone(), gpu.queue.clone(), editor.exec.clone());
        let gate = gz_app::gpu_gate(editor);
        let made = editor
            .exec
            .cpu(move || Renderer::new(device, queue, gate, exec).map(Arc::new).map_err(|e| e.to_string()))
            .await;
        if let Ok(r) = &made {
            // the window stops the render thread while its loop still runs
            let r = r.clone();
            gz_app::GpuOwners::of(editor).on_exit("the thumbnails' renderer", move || r.shutdown());
        }
        *slot = Some(made);
    }
    slot.clone().unwrap_or_else(|| Err("the renderer did not start".to_owned()))
}

const THUMB_SIDE: u32 = 256;

/// A model drawn offscreen at the three-quarter view, framed: what a game mesh shows instead of a
/// baked thumbnail.
async fn render_model(editor: &Arc<Editor>, name: &str, source: ModelSource) -> Result<Vec<u8>, String> {
    let data = load_model(editor.clone(), name.to_owned(), source).await;
    if !data.shown() {
        return Err(data.error.clone());
    }
    let renderer = thumb_renderer(editor).await?;
    let p = gz_ui::Palette::new(gz_ui::Accent::Violet);
    let bg = gz_render::Color::rgb(p.canvas.r(), p.canvas.g(), p.canvas.b());
    editor
        .exec
        .io(move || {
            let assets = renderer.assets();
            if let Some(s) = &data.sources {
                assets.set_loader(Some(Arc::new(s.clone())));
            }
            let Some(mesh) = &data.mesh else { return Err("the model has no geometry to draw".to_owned()) };
            let handle = assets.add_mesh("gz-ui-previews\\thumbnail", mesh.clone());
            let mut list = gz_render::DrawList::new();
            let mut inst = Instance::new(handle, glam::Mat4::IDENTITY)
                .with_flags(InstanceFlags::NO_ID | InstanceFlags::NO_CULL);
            inst.bounds = Some(data.bounds);
            if let Some(pose) = &data.pose {
                inst = inst.with_pose(pose.clone());
            }
            list.push(inst);
            let size = [THUMB_SIDE, THUMB_SIDE];
            let mut view = View::new(Orbit::new(&data.bounds).camera(), size);
            view.flags = ViewFlags::PREVIEW;
            view.background = bg;
            assets.prepare(&list, Duration::from_secs(10));
            let packet = build_packet(&list, &view, assets);
            let img = renderer.render_offscreen(&packet, size).map_err(|e| e.to_string())?;
            gz_app::encode_png(img.width, img.height, &img.rgba).map_err(|e| e.message)
        })
        .await
}

fn png_of_pixels(side: u32, rgba: &[u8]) -> Result<Vec<u8>, String> {
    gz_app::encode_png(side, side, rgba).map_err(|e| e.message)
}

/// A baked thumbnail of a library category: an object's or a group's `.thm` picture, a
/// texture's tile.
fn category_thumbnail(sources: &crate::loader::Sources, category: &str, name: &str) -> Option<Vec<u8>> {
    let thm_pixels = |path: std::path::PathBuf| -> Option<Vec<u8>> {
        let bytes = std::fs::read(path).ok()?;
        let pixels = match gz_fmt_tex::read_thm(&bytes).ok()? {
            gz_fmt_tex::Thm::Object(o) => o.pixels,
            gz_fmt_tex::Thm::Group(g) => g.pixels,
            gz_fmt_tex::Thm::Texture(t) => t.pixels?,
            _ => return None,
        };
        png_of_pixels(gz_fmt_tex::THUMB_SIDE, &pixels).ok()
    };
    let rel = name.replace('/', "\\");
    match category {
        "Objects" => {
            let mut dirs = Vec::new();
            if let Some(p) = &sources.project {
                dirs.push(p.join("rawdata").join("objects"));
            }
            dirs.push(sources.dirs.objects.clone());
            dirs.into_iter().find_map(|d| thm_pixels(d.join(format!("{rel}.thm"))))
        }
        "Groups" => {
            let mut dirs = Vec::new();
            if let Some(p) = &sources.project {
                dirs.push(p.join("rawdata").join("groups"));
            }
            dirs.push(sources.dirs.root.join("rawdata").join("groups"));
            dirs.into_iter().find_map(|d| thm_pixels(d.join(format!("{rel}.thm"))))
        }
        "Textures" | "Textures (raw)" => {
            let path = if category == "Textures" {
                sources.dirs.game_textures.join(format!("{rel}.dds"))
            } else {
                sources.dirs.textures.join(format!("{rel}.tga"))
            };
            let bytes = std::fs::read(&path).ok()?;
            let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase());
            let tile = gz_fmt_tex::texture_tile(&bytes, ext.as_deref(), gz_fmt_tex::THUMB_SIDE).ok()?;
            gz_app::encode_png(tile.image.width, tile.image.height, &tile.image.rgba).ok()
        }
        _ => None,
    }
}

fn png_reply(name: &str, png: &[u8], rendered: bool) -> Value {
    let mut out = Map::new();
    if rendered {
        out.insert("rendered".into(), json!(true));
    }
    out.insert("name".into(), json!(crate::paths::json_path_str(name)));
    out.insert("png_base64".into(), json!(base64::engine::general_purpose::STANDARD.encode(png)));
    Value::Object(out)
}

async fn asset_shot(editor: Arc<Editor>, p: AssetShotParams, _origin: Origin) -> Reply {
    let name = engine_name(&p.name);
    if name.is_empty() {
        return Err(invalid("name names the asset (see file.list source=library)"));
    }
    let category = match p.kind.as_deref().unwrap_or("objects").trim().to_ascii_lowercase().as_str() {
        "objects" | "object" => "Objects",
        "groups" | "group" => "Groups",
        "textures" | "texture" => "Textures",
        "raw_textures" => "Textures (raw)",
        "visuals" | "visual" => "Visuals",
        _ => return Err(invalid("kind takes objects, groups, textures, raw_textures or visuals")),
    };
    if category == "Visuals" && p.source.as_deref() == Some("game") {
        let sources = crate::loader::Sources::of(&editor).await;
        if sources.game.is_none() {
            return Err(invalid("no game is linked: link one with game.link"));
        }
        if sources.find_game_visual(&name).is_none() {
            return Err(invalid("not found in the game's files (see file.list source=game)"));
        }
        let png = render_model(&editor, &name, ModelSource::Game)
            .await
            .map_err(|_| invalid("cannot render a preview for this model"))?;
        return Ok(png_reply(&name, &png, true));
    }
    let sources = crate::loader::Sources::of(&editor).await;
    let (n, c) = (name.clone(), category);
    let baked = editor.exec.io(move || category_thumbnail(&sources, c, &n)).await;
    let png = match baked {
        Some(p) => p,
        // the visual hook answers through a per-frame queue: a one-shot call renders instead
        None if matches!(category, "Visuals" | "Objects") => {
            render_model(&editor, &name, ModelSource::Library)
                .await
                .map_err(|_| invalid("no preview for this asset (missing thumbnail or wrong name)"))?
        }
        None => return Err(invalid("no preview for this asset (missing thumbnail or wrong name)")),
    };
    Ok(png_reply(&name, &png, false))
}

/// Registers `preview.model`, `preview.texture`, the library commands and `shot.asset`.
pub fn register(reg: &mut Registry) {
    reg.area("preview", "The model and texture previews: one asset in a tab of its own");
    reg.cmd(
        Spec::view("preview.model", "Shows a model in the model preview, or changes how it is shown.")
            .example("name=dynamics\\box\\box_1a lighting=unlit"),
        preview_model,
    );
    reg.cmd(
        Spec::view("preview.texture", "Shows a texture in the texture preview, or changes how it is shown.")
            .example("action=open path=ui\\ui_icon source=library zoom=fit"),
        texture_preview,
    );
    reg.cmd(
        Spec::view("library.view", "Opens a library tab, picks its entry or answers what it shows.")
            .example("kind=texture_library action=select item=wood_table"),
        library,
    );
    reg.cmd(
        Spec::write(
            "library.set",
            "Changes what a library entry is described by: a texture's description, a sound's parameters, a light animation's properties and keys.",
        )
        .example("kind=texture_library item=wpn\\wpn_ak74 set={\"format\":\"DXT5\",\"mips\":false}"),
        library_set,
    );
    reg.cmd(
        Spec::view("library.play", "Plays or stops a sound's game sound or a light animation in its tab.")
            .example("kind=sound_library action=play item=weapons\\ak74_shoot"),
        library_play,
    );
    reg.cmd(
        Spec::write(
            "library.import",
            "Brings pictures in as textures of the library: copied into the import folder and imported, as a job.",
        )
        .destroys_with(&["overwrite"])
        .example("from=D:\\art\\wood_table.png"),
        library_import,
    );
    reg.cmd(
        Spec::write(
            "library.rebuild",
            "Makes the game texture or the game sound of an entry again from its source.",
        )
        .example("kind=sound_library item=weapons\\ak74_shoot"),
        library_rebuild,
    );
    reg.cmd(
        Spec::read("shot.asset", "A picture of an asset: its thumbnail, or the model rendered.")
            .example("name=props\\box_wood_01 kind=objects"),
        asset_shot,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parameters_read_what_a_caller_writes() {
        let m: ModelParams =
            serde_json::from_value(json!({ "lighting": "unlit", "wireframe": true })).unwrap();
        assert_eq!((m.lighting, m.wireframe), (Some(Lighting::Unlit), Some(true)));
        let t: TextureParams = serde_json::from_value(json!({ "action": "open", "mip": 2 })).unwrap();
        assert_eq!((t.action, t.mip), (TextureAction::Open, Some(2)));
        assert!(
            serde_json::from_value::<LibraryParams>(json!({ "kind": "texture_library", "z": 1 })).is_err()
        );
    }
}
