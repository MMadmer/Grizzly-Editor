//! The commands of the previews: `preview.model`, `preview.texture`, `library.view` and
//! `shot.asset`.
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
    let source_word = p.source.clone().unwrap_or_else(|| "library".to_owned());
    if !matches!(source_word.as_str(), "library" | "project" | "game") {
        return Err(invalid("source takes library, project or game"));
    }
    let source = ModelSource::parse(&source_word);
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
            if source == ModelSource::Game
                && loaded.as_ref().is_some_and(|d| d.error == "not found in the linked game install")
            {
                return Err(invalid("not found in the linked game install"));
            }
            editor.services.get_or_init(ModelPreload::default).put(&name, source, pending);
            open_tab(&editor, "model", Some(&name), Some(&source_word)).await?;
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

// ---- library.view ------------------------------------------------------------------------------

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
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct LibraryParams {
    /// light_animations, texture_library or sound_library.
    kind: String,
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
}

async fn library(editor: Arc<Editor>, p: LibraryParams, _origin: Origin) -> Reply {
    let kind = p.kind.clone();
    if !KINDS.contains(&kind.as_str()) {
        return Err(invalid("kind must be light_animations, texture_library or sound_library"));
    }
    let action = match p.action {
        LibraryAction::State => "state",
        LibraryAction::Open => "open",
        LibraryAction::Select => "select",
    }
    .to_owned();
    // what the tabs read: their own words for the same things
    let mut args = Args::new();
    if let Some(i) = &p.item {
        args.insert("item".into(), json!(i));
    }
    if let Some(f) = p.frame {
        args.insert("frame".into(), json!(f));
    }
    if let Some(q) = &p.query {
        args.insert("search".into(), json!(q));
    }
    // open brings the tab up; state and select never switch tabs
    if action == "open" {
        let k = kind.clone();
        let there =
            on_ui(&editor, move |shell, ctx| Ok(shell.document_mut(&k).is_some() && shell.activate(ctx, &k)))
                .await?;
        if !there {
            open_tab(&editor, &kind, None, None).await?;
        }
    }
    let open = {
        let k = kind.clone();
        on_ui(&editor, move |shell, _| Ok(shell.document_mut(&k).is_some())).await?
    };
    if !open {
        return Err(invalid(format!("the {kind} tab is not open: action=open opens it")));
    }
    // what the tab answers about is its list as read
    match kind.as_str() {
        "light_animations" => {
            if let Some(store) =
                on_ui(&editor, |shell, _| Ok(doc::<LanimDoc>(shell, "light_animations").map(|d| d.store())))
                    .await?
            {
                LanimDoc::loaded(&store).await;
            }
        }
        "texture_library" => {
            let import = on_ui(&editor, |shell, _| {
                Ok(doc::<TexLibDoc>(shell, "texture_library").and_then(|d| d.import_pending()))
            })
            .await?;
            TexLibDoc::items_now(&editor, import).await;
        }
        _ => {
            SoundLibDoc::items_now(&editor).await;
        }
    }
    let run = |editor: Arc<Editor>, kind: String, action: String, args: Args| async move {
        on_ui(&editor, move |shell, _| {
            let active = shell.active() == kind;
            let d: Option<&mut dyn LibraryDoc> = match kind.as_str() {
                "light_animations" => doc::<LanimDoc>(shell, &kind).map(|d| d as &mut dyn LibraryDoc),
                "texture_library" => doc::<TexLibDoc>(shell, &kind).map(|d| d as &mut dyn LibraryDoc),
                _ => doc::<SoundLibDoc>(shell, &kind).map(|d| d as &mut dyn LibraryDoc),
            };
            let Some(d) = d else {
                return Err(invalid(format!("the {kind} tab is not open: action=open opens it")));
            };
            let body = d.mcp(&action, &args).map_err(invalid)?;
            let settle = if action == "select" { d.settle() } else { None };
            Ok((active, body, settle))
        })
        .await
    };
    let (mut active, mut body, settle) = run(editor.clone(), kind.clone(), action.clone(), args).await?;
    // a selection answers once what it shows is read: the picture, the game sound
    if let Some(s) = settle {
        s.await;
        (active, body, _) = run(editor.clone(), kind.clone(), "state".to_owned(), Args::new()).await?;
    }
    let mut head = Map::new();
    head.insert("tab".into(), json!(kind));
    head.insert("active".into(), json!(active));
    if active {
        owe_frames(&editor, 2).await;
    }
    Ok(merge(head, std::mem::take(&mut body)))
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
            render_model(&editor, &name, ModelSource::Visual)
                .await
                .map_err(|_| invalid("no preview for this asset (missing thumbnail or wrong name)"))?
        }
        None => return Err(invalid("no preview for this asset (missing thumbnail or wrong name)")),
    };
    Ok(png_reply(&name, &png, false))
}

/// Registers `preview.model`, `preview.texture`, `library.view` and `shot.asset`.
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
            serde_json::from_value::<LibraryParams>(json!({ "kind": "texture_library", "x": 1 })).is_err()
        );
    }
}
