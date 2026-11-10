//! The commands of the previews: `preview_model`, `texture_preview`, `library`, `asset_preview`
//! (`LevelMcpPreview.cpp`, `EditorLibraryMcp`, `LevelMcpContent.cpp`). The wire is the C++ one.
//!
//! A command that changes what a tab draws applies itself, then waits for the frames that show
//! it and reads the tab again: the answer is the frame it asked for, `pending` is always false.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use gz_app::on_ui;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Origin, Reply};
use gz_render::{Instance, InstanceFlags, Renderer, View, ViewFlags, build_packet};
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

/// `ArgFlag`: `None` when the argument is not there.
fn flag(args: &Args, key: &str) -> Option<bool> {
    if !args.has(key) {
        return None;
    }
    Some(args.bool(key).unwrap_or(false))
}

/// The frames an answer owes: drawn, and read again. Without a window there are none.
async fn owe_frames(editor: &Editor, n: u64) {
    if editor.frames.has_window() {
        let _ = editor.frames.after(n).await;
    }
}

/// Opens a tab through the shell's own opener (`ui_tab open`), so a person and an agent share
/// one path; an error reply becomes this command's error.
async fn open_tab(editor: &Arc<Editor>, kind: &str, path: Option<&str>, source: Option<&str>) -> Result<(), CmdError> {
    let mut args = Args::new();
    args.insert("action".into(), json!("open"));
    args.insert("kind".into(), json!(kind));
    if let Some(p) = path {
        args.insert("path".into(), json!(p));
    }
    if let Some(s) = source {
        args.insert("source".into(), json!(s));
    }
    let reply = editor.call("ui_tab", args, Origin::Mcp).await;
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

// ---- preview_model -----------------------------------------------------------------------------

/// What the model tab answers: `tab`, `active`, its state, `pending`.
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
    let mut tail = state.unwrap_or_else(|| Map::from_iter([("open".to_owned(), json!(false))]));
    tail.insert("pending".into(), json!(false));
    Ok(merge(head, tail))
}

async fn preview_model(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let name = engine_name(args.str("name").unwrap_or_default());
    let source = ModelSource::parse(args.str("source").unwrap_or_default());
    let lighting = match args.str("lighting").unwrap_or_default() {
        "" => None,
        "studio" => Some(0),
        "unlit" => Some(1),
        _ => return Err(invalid("lighting must be studio or unlit")),
    };
    let (wireframe, bones, details) = (flag(&args, "wireframe"), flag(&args, "bones"), flag(&args, "details"));
    let (frame, reset) = (args.flag("frame"), args.flag("reset"));
    if args.flag("answer_only") {
        return model_answer(&editor).await;
    }

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
            if source == ModelSource::Game && loaded.as_ref().is_some_and(|d| d.error == "not found in the linked game install") {
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

    let asked = !name.is_empty() || lighting.is_some() || wireframe.is_some() || bones.is_some() || details.is_some() || frame || reset;
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
            let before = on_ui(&editor, |shell, _| Ok(doc::<ModelDoc>(shell, "model").map_or(0, |d| d.redraw()))).await?;
            let mut serial = before;
            for _ in 0..30 {
                owe_frames(&editor, 1).await;
                serial = on_ui(&editor, |shell, _| Ok(doc::<ModelDoc>(shell, "model").map_or(0, |d| d.submitted()))).await?;
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

// ---- texture_preview ---------------------------------------------------------------------------

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
    let mut tail = state.unwrap_or_else(|| Map::from_iter([("open".to_owned(), json!(false))]));
    tail.insert("pending".into(), json!(false));
    Ok(merge(head, tail))
}

async fn texture_preview(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let action = args.str_or("action", "state").to_owned();
    if args.flag("answer_only") {
        return texture_answer(&editor).await;
    }
    let mut asked = false;
    match action.as_str() {
        "open" => {
            let path = args.str("path").unwrap_or_default().replace('/', "\\");
            let source = TexSource::parse(args.str("source").unwrap_or_default());
            let Some(source) = source.filter(|_| !path.is_empty()) else { return Err(invalid("open takes path= and source=project|editor|darf")) };
            let (p, s) = (path.clone(), source);
            let showing = on_ui(&editor, move |shell, ctx| {
                let same = doc::<TextureDoc>(shell, "texture").is_some_and(|d| d.name() == p && d.source() == s && d.data().is_some_and(|t| t.texture.is_some()));
                if same {
                    shell.activate(ctx, "texture");
                }
                Ok(same)
            })
            .await?;
            if !showing {
                let pending = Pending::spawn(&editor.exec, load_texture(editor.clone(), path.clone(), source));
                let loaded = pending.wait(LOAD_WAIT).await;
                editor.services.get_or_init(TexturePreload::default).put(&path, source, pending);
                open_tab(&editor, "texture", Some(&path), Some(source.arg())).await?;
                if let Some(t) = loaded
                    && !t.error.is_empty()
                {
                    return Err(invalid(if t.error == UNREADABLE { UNREADABLE_REPLY.to_owned() } else { t.error.clone() }));
                }
            }
            asked = true;
        }
        "close" => {
            let open = on_ui(&editor, |shell, _| Ok(shell.document_mut("texture").is_some())).await?;
            if open {
                let mut a = Args::new();
                a.insert("action".into(), json!("close"));
                a.insert("id".into(), json!("texture"));
                a.insert("discard".into(), json!(true));
                let _ = editor.call("ui_tab", a, Origin::Mcp).await;
            }
            return texture_answer(&editor).await;
        }
        "state" => {}
        _ => return Err(invalid("action must be open, state or close")),
    }
    let open = on_ui(&editor, |shell, _| Ok(shell.document_mut("texture").is_some())).await?;
    if !open {
        return texture_answer(&editor).await;
    }
    let zoom_text = args.str("zoom").unwrap_or_default().to_owned();
    let zoom = match zoom_text.as_str() {
        "" => None,
        "fit" => Some(0.0),
        z => Some(z.trim().parse::<f32>().unwrap_or(0.0)),
    };
    let channel_text = args.str("channel").unwrap_or_default();
    let channel = if channel_text.is_empty() { None } else { parse_channel(channel_text).map(Some).unwrap_or(Some(usize::MAX)) };
    if channel == Some(usize::MAX) || (zoom_text != "fit" && zoom.is_some_and(|z| z <= 0.0 || !z.is_finite())) {
        return Err(invalid("zoom is fit or a factor, channel rgba, r, g, b or a"));
    }
    let mip = args.i64("mip").filter(|m| *m >= 0).map(|m| u32::try_from(m).unwrap_or(u32::MAX));
    let details = flag(&args, "details");
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

// ---- library -----------------------------------------------------------------------------------

async fn library(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let kind = args.str("kind").unwrap_or_default().to_owned();
    if !KINDS.contains(&kind.as_str()) {
        return Err(invalid("kind must be light_animations, texture_library or sound_library"));
    }
    let action = args.str_or("action", "state").to_owned();
    if !matches!(action.as_str(), "open" | "state" | "select") {
        return Err(invalid("action must be open, state or select"));
    }
    // open brings the tab up; state and select never switch tabs
    if action == "open" {
        let k = kind.clone();
        let there = on_ui(&editor, move |shell, ctx| Ok(shell.document_mut(&k).is_some() && shell.activate(ctx, &k))).await?;
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
            if let Some(store) = on_ui(&editor, |shell, _| Ok(doc::<LanimDoc>(shell, "light_animations").map(|d| d.store()))).await? {
                LanimDoc::loaded(&store).await;
            }
        }
        "texture_library" => {
            let import = on_ui(&editor, |shell, _| Ok(doc::<TexLibDoc>(shell, "texture_library").and_then(|d| d.import_pending()))).await?;
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
            let Some(d) = d else { return Err(invalid(format!("the {kind} tab is not open: action=open opens it"))) };
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

// ---- asset_preview -----------------------------------------------------------------------------

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
        let made = editor.exec.cpu(move || Renderer::new(device, queue, exec).map(Arc::new).map_err(|e| e.to_string())).await;
        *slot = Some(made);
    }
    slot.clone().unwrap_or_else(|| Err("the renderer did not start".to_owned()))
}

const THUMB_SIDE: u32 = 256;

/// A model drawn offscreen at the three-quarter view, framed: what a game mesh shows instead of a
/// baked thumbnail (`RenderVisualThumbnail`, `RenderObjectThumbnail`).
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
            let mut inst = Instance::new(handle, glam::Mat4::IDENTITY).with_flags(InstanceFlags::NO_ID | InstanceFlags::NO_CULL);
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
            let path = if category == "Textures" { sources.dirs.game_textures.join(format!("{rel}.dds")) } else { sources.dirs.textures.join(format!("{rel}.tga")) };
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

async fn asset_preview(editor: Arc<Editor>, args: Args, _origin: Origin) -> Reply {
    let Some(raw) = args.text("name") else { return Err(invalid("missing 'name' argument (see list_assets)")) };
    let name = engine_name(raw);
    let source = args.str("source").unwrap_or_default().to_lowercase();
    match source.as_str() {
        "object" => {
            let png = render_model(&editor, &name, ModelSource::Visual).await.map_err(|_| invalid("cannot render this library object"))?;
            return Ok(png_reply(&name, &png, true));
        }
        "visual" | "darf" => {
            let src = ModelSource::parse(&source);
            if src == ModelSource::Game {
                let sources = crate::loader::Sources::of(&editor).await;
                if sources.game.is_none() {
                    return Err(invalid("no game is linked: link one with game_link"));
                }
                if sources.find_game_visual(&name).is_none() {
                    return Err(invalid("not found in the linked game install (see darf_list)"));
                }
            }
            let png = render_model(&editor, &name, src).await.map_err(|_| invalid("cannot render a preview for this model"))?;
            return Ok(png_reply(&name, &png, true));
        }
        _ => {}
    }
    let category = match args.text("category") {
        None => "Objects",
        Some(c) => match gz_project::CATEGORIES.iter().find(|n| n.eq_ignore_ascii_case(c)) {
            Some(n) => *n,
            None => return Err(invalid("unknown category")),
        },
    };
    if !matches!(category, "Objects" | "Groups" | "Textures" | "Textures (raw)" | "Visuals") {
        return Err(invalid("category has no thumbnails in this build"));
    }
    let sources = crate::loader::Sources::of(&editor).await;
    let (n, c) = (name.clone(), category);
    let baked = editor.exec.io(move || category_thumbnail(&sources, c, &n)).await;
    let png = match baked {
        Some(p) => p,
        // the visual hook answers through a per-frame queue: a one-shot call renders instead
        None if matches!(category, "Visuals" | "Objects") => render_model(&editor, &name, ModelSource::Visual).await.map_err(|_| invalid("no preview for this asset (missing thumbnail or wrong name)"))?,
        None => return Err(invalid("no preview for this asset (missing thumbnail or wrong name)")),
    };
    Ok(png_reply(&name, &png, false))
}

pub fn register(reg: &mut gz_editor::Registry) {
    reg.add(crate::AREA, "preview_model", preview_model);
    reg.add(crate::AREA, "texture_preview", texture_preview);
    reg.add(crate::AREA, "library", library);
    reg.add(crate::AREA, "asset_preview", asset_preview);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_are_absent_or_read() {
        let mut a = Args::new();
        assert_eq!(flag(&a, "wireframe"), None);
        a.insert("wireframe".into(), json!("true"));
        assert_eq!(flag(&a, "wireframe"), Some(true));
        a.insert("bones".into(), json!(0));
        assert_eq!(flag(&a, "bones"), Some(false));
    }
}
