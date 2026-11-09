//! Pictures of the editor over MCP.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply, json};

use crate::window::WindowLink;

const AREA: &str = "window";

/// Encodes straight RGBA as a PNG. Fast settings: a screenshot is looked at, not archived.
pub fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, CmdError> {
    let mut out = Vec::with_capacity(rgba.len() / 4);
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().map_err(|e| CmdError::internal(format!("png encode failed for {width}x{height}: {e}")))?;
        writer.write_image_data(rgba).map_err(|e| CmdError::internal(format!("png encode failed for {width}x{height}: {e}")))?;
    }
    Ok(out)
}

/// One call, always current: the request is held for the frames it asked for, then pictured.
async fn screenshot_editor(editor: Arc<Editor>, args: Args) -> Reply {
    let link = editor.services.get::<WindowLink>().ok_or_else(|| CmdError::new("no_window", "the editor runs without a window"))?;
    // the commands that came before this one have to be drawn first: a layout settles in two passes
    let frames = args.u32("frames").unwrap_or(2).clamp(1, 16);
    editor.frames.after(u64::from(frames)).await?;

    let (tx, rx) = tokio::sync::oneshot::channel();
    link.capture(move |image| {
        let _ = tx.send(image);
    });
    editor.exec.wake().ping();
    let image = tokio::time::timeout(Duration::from_secs(10), rx)
        .await
        .map_err(|_| CmdError::new("no_frame", "the editor did not draw a frame in time"))?
        .map_err(|_| CmdError::internal("the picture was lost"))?;

    let stamp = editor.frames.current();
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
        "dpi": stamp.scale,
        "active_tab": stamp.active_tab,
        "frame": stamp.frame,
    }))
}

/// Registers the window's own commands.
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "screenshot_editor", |editor, args, _| screenshot_editor(editor, args));
}
