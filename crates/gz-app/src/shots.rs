//! Pictures of the editor over MCP.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply, Value, json};
use gz_ui::dialog;

use crate::contract;
use crate::queue::UiQueue;
use crate::window::WindowLink;

const AREA: &str = "window";

/// What a picture of the window holds for besides the UI frames: the viewport's pass of the scene
/// as the caller's commands left it, with what it draws loaded (the viewport crate fills it in).
/// It answers what was still pending when its limit ran out.
type GateFn = Arc<dyn Fn(Arc<Editor>) -> futures::future::BoxFuture<'static, Vec<String>> + Send + Sync>;

#[derive(Default)]
pub struct PictureGate {
    gate: parking_lot::Mutex<Option<GateFn>>,
}

impl PictureGate {
    pub fn of(editor: &Editor) -> Arc<PictureGate> {
        editor.services.get_or_init(PictureGate::default)
    }

    pub fn set(
        &self,
        gate: impl Fn(Arc<Editor>) -> futures::future::BoxFuture<'static, Vec<String>> + Send + Sync + 'static,
    ) {
        *self.gate.lock() = Some(Arc::new(gate));
    }

    fn get(&self) -> Option<GateFn> {
        self.gate.lock().clone()
    }
}

/// Encodes straight RGBA as a PNG. Fast settings: a screenshot is looked at, not archived.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, CmdError> {
    let mut out = Vec::with_capacity(rgba.len() / 4);
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|e| CmdError::internal(format!("png encode failed for {width}x{height}: {e}")))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| CmdError::internal(format!("png encode failed for {width}x{height}: {e}")))?;
    }
    Ok(out)
}

/// The jobs that were the C++ editor's blocking operations (`SProgressOperation`): a level
/// build, a scene load, a scene tool, the texture and sound libraries' work.
const BLOCKING_KINDS: [&str; 7] = [
    "level_build",
    "scene_load",
    "scene_tool",
    "sync_textures",
    "sync_sounds",
    "texture_library",
    "sound_library",
];

/// `busy` as `AppendShotWindows` defines it (`ProgressOperationActive`): one of the operations
/// the C++ ran as a blocking job is running while the picture is taken. They run beside the
/// window here, so the frame is drawn as usual.
fn busy_now(editor: &Editor) -> bool {
    editor.jobs.snapshot().iter().any(|j| j.running() && BLOCKING_KINDS.contains(&j.kind.as_str()))
}

/// One call, always current: the request is held for the frames it asked for, then pictured.
async fn screenshot_editor(editor: Arc<Editor>, args: Args) -> Reply {
    let link = editor
        .services
        .get::<WindowLink>()
        .ok_or_else(|| CmdError::new("no_window", "the editor runs without a window"))?;
    // the commands that came before this one have to be drawn first: a layout settles in two passes
    let frames = args.u32("frames").unwrap_or(2).clamp(1, 16);
    editor.frames.after(u64::from(frames)).await?;
    // the viewport's picture of the scene is part of the frame: held until it is current too,
    // then two more frames that show it
    let pending = match PictureGate::of(&editor).get() {
        Some(gate) => {
            let pending = gate(editor.clone()).await;
            editor.frames.after(2).await?;
            pending
        }
        None => Vec::new(),
    };

    let (tx, rx) = tokio::sync::oneshot::channel();
    link.capture(move |image| {
        let _ = tx.send(image);
    });
    editor.exec.wake().ping();
    let image = tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .map_err(|_| CmdError::new("no_frame", "the editor did not draw a frame in time"))?
        .map_err(|_| CmdError::internal("the picture was lost"))?;

    // the modal on top of the frame just pictured: a kit dialog (its `dlg.<name>`), else the palette
    let modal = match UiQueue::of(&editor) {
        Ok(q) => q
            .run(|shell, ctx| {
                if let Some(d) = dialog::modal_on_top(ctx) {
                    let [x0, y0, x1, y1] = d.rect_px;
                    return Ok(json!({ "title": d.title, "id": d.id, "x": x0.floor() as i64, "y": y0.floor() as i64, "w": (x1 - x0).floor() as i64, "h": (y1 - y0).floor() as i64 }));
                }
                if shell.palette_open() {
                    let ppp = ctx.pixels_per_point();
                    let r = shell.palette_rect();
                    return Ok(json!({ "title": "Command Palette", "id": contract::PALETTE_ID, "x": (r.min.x * ppp).floor() as i64, "y": (r.min.y * ppp).floor() as i64, "w": (r.width() * ppp).floor() as i64, "h": (r.height() * ppp).floor() as i64 }));
                }
                Ok(Value::Null)
            })
            .await
            .unwrap_or(Value::Null),
        Err(_) => Value::Null,
    };

    let stamp = editor.frames.current();
    let busy = busy_now(&editor);
    let outside = link.outside();
    let [w, h] = image.size;
    let png = editor
        .exec
        .cpu(move || {
            let mut rgba = Vec::with_capacity(w * h * 4);
            for px in &image.pixels {
                // the frame is opaque: premultiplied and straight are the same bytes
                rgba.extend_from_slice(&[px.r(), px.g(), px.b(), 255]);
            }
            encode_png(w as u32, h as u32, &rgba)
        })
        .await?;
    let encoded = editor.exec.cpu(move || base64::engine::general_purpose::STANDARD.encode(png)).await;
    Ok(json!({
        "png_base64": encoded,
        "width": w,
        "height": h,
        "active_tab": stamp.active_tab,
        "busy": busy,
        "dpi": (f64::from(stamp.scale) * 1000.0).round() / 1000.0,
        "outside": outside,
        "modal": modal,
        "frame": stamp.frame,
    }))
    .map(|mut v| {
        // what the picture still waited for when its limit ran out: never a silent half picture
        if !pending.is_empty()
            && let Some(o) = v.as_object_mut()
        {
            o.insert("pending".to_owned(), json!(pending));
        }
        v
    })
}

/// Registers the window's own commands.
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "screenshot_editor", |editor, args, _| screenshot_editor(editor, args));
}
