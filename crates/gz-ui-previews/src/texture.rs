//! The texture preview: the `texture` tab (docs/UI_DESIGN.md, section 6, "Texture Preview").
//!
//! The file is read and uploaded on the pools ([`crate::texture_load`]); a frame only registers
//! the level on show with egui and draws it: Fit, 1:1, Zoom | RGBA / R / G / B / A | Mip |
//! Details, pan and zoom at the cursor, a checker clipped to the view.

use std::sync::Arc;

use egui::{Align, CornerRadius, CursorIcon, Layout, Rect, ScrollArea, Sense, Ui, UiBuilder, Vec2, pos2};
use gz_app::{Chord, DocCx, Document, OpenRequest};
use gz_editor::Editor;
use gz_ui::grid::read_only;
use gz_ui::splitter::splitter_tail;
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Kind, MenuItem, PropFlags, PropertyGrid, Toolbar, menu_item, palette, state};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::gpu::mip_view;
use crate::image_pass::{ImageDraw, ImageMode, paint_image};
use crate::local::local_checker;
use crate::pending::Pending;
use crate::texture_load::{TexFile, TexSource, gpu_bytes, human_bytes, load};

pub const DETAILS_W: f32 = 320.0;
pub const SIDE_MIN: f32 = 220.0;
pub const CANVAS_MIN: f32 = 400.0;
pub const ZOOM_MIN: f32 = 1.0 / 64.0;
pub const ZOOM_MAX: f32 = 32.0;

pub const CHANNELS: [&str; 5] = ["RGBA", "R", "G", "B", "A"];
pub const CHANNEL_IDS: [&str; 5] = ["rgba", "r", "g", "b", "a"];
const ZOOM_STEPS: [f32; 8] = [0.125, 0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0];

/// What `channel=` names: `rgba` (or the first version's `rgb`), `r`, `g`, `b`, `a`.
#[must_use]
pub fn parse_channel(text: &str) -> Option<usize> {
    match text {
        "rgba" | "rgb" => Some(0),
        "r" => Some(1),
        "g" => Some(2),
        "b" => Some(3),
        "a" => Some(4),
        _ => None,
    }
}

/// Where the image lands in the canvas: fit keeps the aspect and never blows a
/// small texture up past 1:1. All in pixels; returns the image's top-left corner, the scale and
/// the pan as it stays.
#[must_use]
pub fn place(avail: Vec2, img: Vec2, zoom: f32, pan: Vec2, keep: f32) -> (egui::Pos2, f32, Vec2) {
    let fit = (avail.x / img.x).min(avail.y / img.y).min(1.0);
    let scale = if zoom > 0.0 { zoom } else { fit };
    let mut pan = if zoom > 0.0 { pan } else { Vec2::ZERO };
    if zoom > 0.0 {
        // some of the image always stays in view
        let mx = ((avail.x + img.x * scale) * 0.5 - keep).max(0.0);
        let my = ((avail.y + img.y * scale) * 0.5 - keep).max(0.0);
        pan = Vec2::new(pan.x.clamp(-mx, mx), pan.y.clamp(-my, my));
    }
    let x = ((avail.x - img.x * scale) * 0.5 + pan.x).floor();
    let y = ((avail.y - img.y * scale) * 0.5 + pan.y).floor();
    (pos2(x, y), scale, pan)
}

/// The texture tab.
pub struct TextureDoc {
    editor: Arc<Editor>,
    name: String,
    source: TexSource,
    load: Pending<TexFile>,
    /// 0 is fit.
    pub zoom: f32,
    /// px.
    pan: Vec2,
    pub channel: usize,
    pub mip: u32,
    pub show_details: bool,
    /// What the last frame drew at: screen pixels per texel.
    pub scale: f32,
    pub details_w: f32,
    drawn_frame: Option<u64>,
    gpu: Option<egui_wgpu::RenderState>,
    /// The level on show, sampled linear and nearest; made again when the level changes.
    draws: Option<(u32, Arc<ImageDraw>, Arc<ImageDraw>)>,
    /// The Reload Textures generation the file was read at.
    reload_seen: u64,
}

impl TextureDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>, name: String, source: TexSource, load: Pending<TexFile>) -> Self {
        Self {
            editor: editor.clone(),
            name,
            source,
            load,
            zoom: 0.0,
            pan: Vec2::ZERO,
            channel: 0,
            mip: 0,
            show_details: true,
            scale: 1.0,
            details_w: 0.0,
            drawn_frame: None,
            gpu: None,
            draws: None,
            reload_seen: gz_app::gpu::TextureReload::generation(editor),
        }
    }

    #[must_use]
    pub fn data(&self) -> Option<Arc<TexFile>> {
        self.load.get()
    }

    #[must_use]
    pub fn pending(&self) -> Pending<TexFile> {
        self.load.clone()
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn source(&self) -> TexSource {
        self.source
    }

    #[must_use]
    pub fn drawn_lately(&self, frame: u64) -> bool {
        self.drawn_frame.is_some_and(|f| f + 1 >= frame)
    }

    fn image(&self) -> Option<Arc<TexFile>> {
        self.data().filter(|d| d.texture.is_some())
    }

    fn fit(&mut self) {
        self.zoom = 0.0;
        self.pan = Vec2::ZERO;
    }

    /// The view asked for over MCP: `zoom` 0 fits; `None` keeps.
    pub fn apply_view(
        &mut self,
        zoom: Option<f32>,
        mip: Option<u32>,
        channel: Option<usize>,
        details: Option<bool>,
    ) -> Result<(), String> {
        if let Some(z) = zoom {
            self.zoom = if z > 0.0 { z.clamp(ZOOM_MIN, ZOOM_MAX) } else { 0.0 };
            self.pan = Vec2::ZERO;
        }
        if let Some(c) = channel {
            self.channel = c.min(4);
        }
        if let Some(d) = details {
            self.show_details = d;
        }
        if let Some(m) = mip
            && let Some(t) = self.image()
        {
            self.mip = m.min(t.mips.saturating_sub(1));
        }
        Ok(())
    }

    /// What `content.view` reveals the texture by: a file of the project, or one of the Library's
    /// textures (below `textures`, with its extension).
    fn reveal_args(&self) -> Option<gz_editor::Args> {
        if self.name.is_empty() {
            return None;
        }
        let name = self.name.replace('/', "\\");
        Some(match self.source {
            TexSource::Project => {
                gz_app::args(&[("action", "reveal"), ("source", "project"), ("path", &name)])
            }
            TexSource::Editor | TexSource::Game => {
                let lower = name.to_ascii_lowercase();
                let rel = if lower.starts_with("textures\\") { &name[9..] } else { name.as_str() };
                let file = if rel.to_ascii_lowercase().ends_with(".dds") {
                    rel.to_owned()
                } else {
                    format!("{rel}.dds")
                };
                gz_app::args(&[
                    ("action", "reveal"),
                    ("source", "library"),
                    ("kind", "textures"),
                    ("path", &file),
                ])
            }
        })
    }

    fn toolbar_items<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>) {
        let img = self.image();
        let on = img.is_some();
        let none = "No texture is open";
        tb.group(3);
        if tb.action(
            "fit",
            Icon::ZoomFit,
            "Fit",
            if on { "Fit the image into the canvas (F)" } else { none },
            on,
            self.zoom <= 0.0,
            Kind::Ghost,
        ) {
            self.fit();
        }
        // the icon already reads "1:1": the label says what it does
        if tb.action(
            "one_to_one",
            Icon::Zoom100,
            "Actual Size",
            if on { "One screen pixel per texel (1:1)" } else { none },
            on,
            self.zoom == 1.0,
            Kind::Ghost,
        ) {
            self.zoom = 1.0;
            self.pan = Vec2::ZERO;
        }
        let zoom = &mut self.zoom;
        let pan = &mut self.pan;
        tb.menu(
            "zoom",
            Icon::ZoomIn,
            "Zoom",
            "Pick a zoom; the wheel zooms at the mouse, any button drags",
            Some("texture_zoom"),
            true,
            |ui| {
                for step in ZOOM_STEPS {
                    let label = format!("{}%", step * 100.0);
                    if menu_item(
                        ui,
                        &label,
                        MenuItem { selected: *zoom == step, enabled: on, ..MenuItem::default() },
                    ) {
                        *zoom = step;
                        *pan = Vec2::ZERO;
                    }
                }
            },
        );
        // in the overflow the channels become radio items
        tb.group(5);
        tb.segmented(
            "channels",
            "Channels",
            &mut self.channel,
            &CHANNELS,
            "RGBA shows the image over a checkerboard; R, G and B one colour channel; A the alpha as grey",
            on,
        );
        tb.group(2);
        let mip = &mut self.mip;
        tb.menu(
            "mip",
            Icon::Picture,
            "Mip",
            "Which mip level the canvas shows",
            Some("texture_mip"),
            true,
            |ui| match &img {
                Some(t) => {
                    for m in 0..t.mips.max(1) {
                        let (w, h) = t.mip_size(m);
                        if menu_item(
                            ui,
                            &format!("Mip {m} \u{b7} {w} x {h}"),
                            MenuItem { selected: m == *mip, ..MenuItem::default() },
                        ) {
                            *mip = m;
                        }
                    }
                }
                None => {
                    menu_item(ui, none, MenuItem { enabled: false, ..MenuItem::default() });
                }
            },
        );
        tb.group(4);
        if tb.action(
            "details",
            Icon::SidebarRight,
            "Details",
            "Show the texture's facts beside the canvas",
            true,
            self.show_details,
            Kind::Ghost,
        ) {
            self.show_details = !self.show_details;
        }
        tb.separator();
        tb.group(1);
        let reveal = self.reveal_args();
        if tb.action(
            "reveal",
            Icon::Reveal,
            "Show in Content Browser",
            if reveal.is_some() { "Select it in the Content Browser" } else { none },
            reveal.is_some(),
            false,
            Kind::Ghost,
        ) && let Some(args) = reveal
        {
            self.editor.post("content.view", args);
        }
    }

    /// The level on show, linear and nearest: views made once per level, the channel views are
    /// the image pass's modes.
    fn draws(&mut self, t: &TexFile) -> Option<(Arc<ImageDraw>, Arc<ImageDraw>)> {
        let tex = t.texture.as_ref()?;
        let gpu = self.gpu.as_ref()?;
        let mip = self.mip.min(t.mips.saturating_sub(1));
        if self.draws.as_ref().is_none_or(|(m, _, _)| *m != mip) {
            let view = mip_view(tex, mip);
            let exec = &self.editor.exec;
            let linear = ImageDraw::new(gpu, &view, false, exec)?;
            let nearest = ImageDraw::new(gpu, &view, true, exec)?;
            self.draws = Some((mip, linear, nearest));
        }
        self.draws.as_ref().map(|(_, a, b)| (a.clone(), b.clone()))
    }

    fn draw_image(&mut self, ui: &mut Ui, rect: Rect, t: &TexFile) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let ppp = ctx.pixels_per_point();
        let resp = ui.interact(rect, ui.scope_id().with("texture.canvas"), Sense::click_and_drag());
        let mip = self.mip.min(t.mips.saturating_sub(1));
        let (mw, mh) = t.mip_size(mip);
        let img = Vec2::new(mw as f32, mh as f32);
        let avail = rect.size() * ppp;
        let keep = 64.0 * ppp;
        let (corner, mut scale, pan) = place(avail, img, self.zoom, self.pan, keep);
        self.pan = pan;
        if resp.hovered() {
            let wheel = crate::local::local_wheel(ui);
            if wheel != 0.0
                && let Some(at) = ctx.pointer_hover_pos()
            {
                let m = (at - rect.min) * ppp;
                let next = (scale * if wheel > 0.0 { 1.25 } else { 1.0 / 1.25 }).clamp(ZOOM_MIN, ZOOM_MAX);
                let texel = (m - corner.to_vec2()) / scale;
                let centred = (avail - img * next) * 0.5;
                self.zoom = next;
                self.pan = m - texel * next - centred;
                scale = next;
            }
        }
        if resp.dragged() {
            if self.zoom <= 0.0 {
                self.zoom = scale;
                self.pan = Vec2::ZERO;
            }
            self.pan += resp.drag_delta() * ppp;
            ctx.set_cursor_icon(CursorIcon::Move);
        }
        if resp.double_clicked() {
            self.fit();
        }
        let (corner, scale, pan) = place(avail, img, self.zoom, self.pan, keep);
        self.pan = pan;
        self.scale = scale;

        let a = rect.min + corner.to_vec2() / ppp;
        let image_rect = Rect::from_min_size(a, img * scale / ppp);
        let clip = image_rect.intersect(rect);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::ZERO, p.canvas);
        if clip.width() <= 0.0 || clip.height() <= 0.0 {
            return;
        }
        // the channel views: RGBA over the checkerboard, one colour over black, the alpha as grey
        if self.channel == 0 {
            local_checker(&painter, &p, image_rect.min, clip);
        }
        // past 2:1 every texel is a block of pixels: sampled nearest, so it reads as one
        if let Some((linear, nearest)) = self.draws(t) {
            let draw = if scale >= 2.0 { nearest } else { linear };
            paint_image(
                &painter,
                image_rect,
                clip,
                [0.0, 0.0, 1.0, 1.0],
                ImageMode::of_channel(self.channel),
                &draw,
            );
        }
        painter.rect_stroke(
            image_rect,
            CornerRadius::ZERO,
            egui::Stroke::new(1.0, p.border_subtle),
            egui::StrokeKind::Outside,
        );
    }

    fn draw_details(&self, ui: &mut Ui) {
        let Some(t) = self.image() else {
            state::empty_state(ui, Icon::Info, "No facts", "Open a texture to see what it is made of.", None);
            return;
        };
        let fi = t.format();
        let ro = PropFlags::READ_ONLY;
        let mip = self.mip.min(t.mips.saturating_sub(1));
        let (mw, mh) = t.mip_size(mip);
        PropertyGrid::new("texture").show(ui, |g| {
            g.section("Texture", true, None, |g| {
                g.row("Name", "The texture's file name", ro, 0, |ui| {
                    read_only(ui, crate::paths::leaf(&t.name), Clip::End)
                });
                g.row("Size", "Its size as imported, in pixels", ro, 0, |ui| {
                    read_only(ui, &format!("{} x {}", t.width, t.height), Clip::End)
                });
                let help = if t.decoded {
                    "The GPU cannot take this file as it is: the preview shows its top level decoded"
                } else {
                    "The file's pixel format, as the GPU holds it"
                };
                g.row("Format", help, ro, 0, |ui| read_only(ui, &t.format_text(), Clip::End));
                // what a decoded copy has is not what the file has: those rows would describe the preview
                if !t.decoded {
                    let yes = |b: bool| if b { "Yes" } else { "No" };
                    g.row("Alpha", "Whether the format keeps an alpha channel", ro, 0, |ui| {
                        read_only(ui, fi.map_or("Unknown", |f| yes(f.alpha)), Clip::End)
                    });
                    g.row("sRGB", "Whether the colours are read as sRGB", ro, 0, |ui| {
                        read_only(ui, fi.map_or("Unknown", |f| yes(f.srgb)), Clip::End)
                    });
                    g.row("Mips", "The mip levels the file holds", ro, 0, |ui| {
                        read_only(ui, &t.mips.to_string(), Clip::End)
                    });
                    if let Some(f) = fi {
                        g.row("GPU Memory", "What the whole mip chain takes on the GPU", ro, 0, |ui| {
                            read_only(ui, &human_bytes(gpu_bytes(&f, t.width, t.height, t.mips)), Clip::End)
                        });
                    }
                }
                if t.file_size > 0 {
                    g.row("On Disk", "The size of the file", ro, 0, |ui| {
                        read_only(ui, &human_bytes(t.file_size), Clip::End)
                    });
                }
                // where it lives and its path are two rows: an ellipsis at the start of one line ate the origin's label
                let (word, hint) = match t.origin.id() {
                    "project" => ("Project", "A file of the project"),
                    "library" => ("Library", "A file of the editor's library (Preferences > Library)"),
                    "game_folder" => ("Game folder", "A loose file under the linked game's gamedata folder"),
                    "game_archive" => {
                        ("Game archive", "The linked game keeps this texture packed in its database archives")
                    }
                    _ => ("", ""),
                };
                if !word.is_empty() {
                    g.row("Source", hint, ro, 0, |ui| read_only(ui, word, Clip::End));
                }
                let path =
                    t.file.as_ref().map_or_else(|| t.archive.clone(), |f| f.to_string_lossy().into_owned());
                if !path.is_empty() {
                    let help = if t.file.is_none() {
                        "Its path below the game's gamedata, inside the archives"
                    } else {
                        "The file the texture was read from"
                    };
                    g.row("Path", help, ro, 0, |ui| read_only(ui, &path, Clip::Start));
                    if let Some(f) = &t.file
                        && g.trailing(Icon::Reveal, "Show in Explorer")
                    {
                        let _ = std::process::Command::new("explorer.exe")
                            .arg(format!("/select,{}", f.display()))
                            .spawn();
                    }
                }
            });
            g.section("View", true, None, |g| {
                g.row("Shown", "The mip level the canvas shows and its size", ro, 0, |ui| {
                    read_only(ui, &format!("Mip {mip} \u{b7} {mw} x {mh}"), Clip::End)
                });
                let zoom = if self.zoom > 0.0 {
                    format!("{:.0}%", self.scale * 100.0)
                } else {
                    format!("Fit ({:.0}%)", self.scale * 100.0)
                };
                g.row("Zoom", "Screen pixels per texel", ro, 0, |ui| read_only(ui, &zoom, Clip::End));
                g.row("Channels", "What the canvas shows of the colour", ro, 0, |ui| {
                    read_only(ui, CHANNELS.get(self.channel).copied().unwrap_or("RGBA"), Clip::End)
                });
            });
        });
    }

    /// `preview.texture`'s fields after `tab` and `active`.
    #[must_use]
    pub fn state_json(&self) -> serde_json::Map<String, Value> {
        let mut out = serde_json::Map::new();
        let t = self.data();
        let t = t.as_deref();
        let shown = t.is_some_and(|t| t.texture.is_some());
        let fi = t.filter(|_| shown).and_then(TexFile::format);
        out.insert("open".into(), json!(true));
        out.insert("name".into(), json!(self.name));
        out.insert(
            "file".into(),
            json!(t.and_then(|t| t.file.as_ref()).map_or_else(String::new, |f| crate::paths::json_path(f))),
        );
        out.insert(
            "archive".into(),
            json!(t.map_or_else(String::new, |t| crate::paths::json_path_str(&t.archive))),
        );
        out.insert("origin".into(), json!(t.map_or("", |t| t.origin.id())));
        out.insert("error".into(), json!(t.map_or("", |t| t.error.as_str())));
        out.insert(
            "format".into(),
            json!(match fi {
                Some(f) => f.name,
                None if shown => "unknown",
                None => "",
            }),
        );
        out.insert("container".into(), json!(t.map_or("", |t| t.container)));
        out.insert("decoded".into(), json!(t.is_some_and(|t| t.decoded)));
        let (w, h, mips) = t.filter(|_| shown).map_or((0, 0, 0), |t| (t.width, t.height, t.mips));
        out.insert("width".into(), json!(w));
        out.insert("height".into(), json!(h));
        out.insert("mips".into(), json!(mips));
        out.insert("mip".into(), json!(self.mip.min(mips.saturating_sub(1))));
        let fit = self.zoom <= 0.0;
        let r3 = |v: f32| (f64::from(v) * 1000.0).round() / 1000.0;
        out.insert("zoom".into(), json!(r3(if fit { self.scale } else { self.zoom })));
        out.insert("fit".into(), json!(fit));
        out.insert("scale".into(), json!(r3(self.scale)));
        out.insert("channel".into(), json!(CHANNEL_IDS.get(self.channel).copied().unwrap_or("rgba")));
        out.insert("alpha".into(), json!(self.channel == 0));
        out.insert("has_alpha".into(), json!(fi.is_some_and(|f| f.alpha)));
        out.insert("bytes".into(), json!(t.map_or(0, |t| t.file_size)));
        out.insert("panes".into(), json!({ "details": self.show_details, "details_width": if self.show_details { self.details_w.round() } else { 0.0 } }));
        out
    }
}

impl Document for TextureDoc {
    fn id(&self) -> &str {
        "texture"
    }
    fn kind(&self) -> &'static str {
        "texture"
    }
    fn label(&self) -> String {
        if self.name.is_empty() { "Texture".to_owned() } else { crate::paths::label_of(&self.name) }
    }
    fn icon(&self) -> Icon {
        Icon::FileTexture
    }
    fn path(&self) -> Option<String> {
        (!self.name.is_empty()).then(|| self.name.clone())
    }
    fn tooltip(&self) -> Option<String> {
        Some(if self.name.is_empty() { "Texture".to_owned() } else { self.name.clone() })
    }
    fn padded(&self) -> bool {
        false
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, _cx: &mut DocCx<'_>) {
        self.toolbar_items(tb);
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let ctx = ui.ctx().clone();
        let frame = ctx.cumulative_frame_nr();
        self.drawn_frame = Some(frame);
        if self.gpu.is_none() {
            self.gpu = Some(cx.gpu.clone());
            // the picture's pass is made while the picture loads
            let _ = crate::image_pass::ImagePass::get(cx.gpu, &cx.editor.exec);
        }
        // Libraries > Reload Textures: the file is read again
        let reload = gz_app::gpu::TextureReload::generation(cx.editor);
        if reload != self.reload_seen {
            self.reload_seen = reload;
            self.load =
                Pending::spawn(&cx.editor.exec, load(cx.editor.clone(), self.name.clone(), self.source));
            self.draws = None;
        }
        let full = ui.max_rect();
        let (canvas, details) = if self.show_details {
            let split = splitter_tail(ui, "texture.details", true, CANVAS_MIN, SIDE_MIN, DETAILS_W);
            self.details_w = split.size;
            let gap = ui.spacing().item_spacing.x;
            let canvas = Rect::from_min_max(
                full.min,
                pos2((full.max.x - split.size - gap).max(full.min.x + 1.0), full.max.y),
            );
            (canvas, Some(Rect::from_min_max(pos2(full.max.x - split.size, full.min.y), full.max)))
        } else {
            self.details_w = 0.0;
            (full, None)
        };
        let p = palette(&ctx);
        match self.data() {
            Some(t) if t.texture.is_some() => self.draw_image(ui, canvas, &t),
            other => {
                ui.painter().rect_filled(canvas, CornerRadius::ZERO, p.canvas);
                let mut child =
                    ui.new_child(UiBuilder::new().max_rect(canvas).layout(Layout::top_down(Align::Center)));
                match other {
                    Some(t) => {
                        state::empty_state(
                            &mut child,
                            Icon::Warning,
                            "This texture cannot be shown",
                            &format!("{}: {}", t.name, t.error),
                            None,
                        );
                    }
                    None => state::busy_line(
                        &mut child,
                        &format!("Reading {}...", crate::paths::leaf(&self.name)),
                    ),
                }
            }
        }
        if let Some(rect) = details {
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt("texture.details.pane")
                    .max_rect(rect.shrink(8.0))
                    .layout(Layout::top_down(Align::Min)),
            );
            ScrollArea::vertical()
                .id_salt("texture.details.scroll")
                .auto_shrink([false, false])
                .show(&mut child, |ui| self.draw_details(ui));
        }
        ui.allocate_rect(full, Sense::hover());
    }

    fn status(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        let Some(t) = self.image() else { return };
        let mip = self.mip.min(t.mips.saturating_sub(1));
        let (w, h) = t.mip_size(mip);
        let line = format!(
            "{w} x {h} \u{b7} {} \u{b7} mip {mip} of {} \u{b7} {:.0}%",
            t.format_text(),
            t.mips,
            self.scale * 100.0
        );
        let p = palette(ui.ctx());
        text::label(ui, &line, Font::Small, p.text_secondary, Clip::End);
    }

    fn shortcut(&mut self, _cx: &mut DocCx<'_>, chord: &Chord) -> bool {
        if chord.key == egui::Key::F && !chord.ctrl && !chord.shift && !chord.alt {
            self.fit();
            return true;
        }
        false
    }

    fn restore_arg(&self) -> Option<String> {
        Some(self.name.clone())
    }

    fn mcp_state(&self) -> Value {
        Value::Object(self.state_json())
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// The texture a command loaded before it opened the tab.
#[derive(Default)]
pub struct TexturePreload {
    slot: Mutex<Option<(String, TexSource, Pending<TexFile>)>>,
}

impl TexturePreload {
    pub fn put(&self, name: &str, source: TexSource, load: Pending<TexFile>) {
        *self.slot.lock() = Some((name.to_owned(), source, load));
    }

    fn take(&self, name: &str, source: TexSource) -> Option<Pending<TexFile>> {
        let mut s = self.slot.lock();
        match s.take() {
            Some((n, src, p)) if n == name && src == source => Some(p),
            other => {
                *s = other;
                None
            }
        }
    }
}

/// The `texture` factory: `path` names the texture, `arg` its source (`project` | `editor` |
/// `darf`).
pub fn factory(editor: &Arc<Editor>, req: &OpenRequest) -> Result<Box<dyn Document>, String> {
    let name = req.path.as_deref().unwrap_or("").replace('/', "\\");
    if name.is_empty() {
        return Err("open takes path= and source=project|library|game".to_owned());
    }
    let source = TexSource::parse(req.arg.as_deref().unwrap_or(""))
        .ok_or_else(|| "source must be project, library or game".to_owned())?;
    let preload = editor.services.get_or_init(TexturePreload::default);
    let pending = preload.take(&name, source).unwrap_or_else(|| {
        let (ed, n) = (editor.clone(), name.clone());
        Pending::spawn(&editor.exec, load(ed, n, source))
    });
    Ok(Box::new(TextureDoc::new(editor, name, source, pending)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_never_blows_up_past_one_to_one() {
        let (_, s, _) = place(Vec2::new(1000.0, 1000.0), Vec2::new(64.0, 64.0), 0.0, Vec2::ZERO, 64.0);
        assert_eq!(s, 1.0);
        let (c, s, _) = place(Vec2::new(100.0, 200.0), Vec2::new(200.0, 200.0), 0.0, Vec2::ZERO, 64.0);
        assert_eq!(s, 0.5);
        assert_eq!(c, pos2(0.0, 50.0));
        // a pan far away is held so some of the image stays in view
        let (_, _, pan) =
            place(Vec2::new(100.0, 100.0), Vec2::new(100.0, 100.0), 1.0, Vec2::new(10_000.0, 0.0), 64.0);
        assert_eq!(pan.x, 36.0);
    }

    #[test]
    fn channels() {
        assert_eq!(parse_channel("rgb"), Some(0));
        assert_eq!(parse_channel("a"), Some(4));
        assert_eq!(parse_channel("x"), None);
    }
}
