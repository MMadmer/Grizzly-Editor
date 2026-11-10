//! Light Animations: the `light_animations` tab over `gamedata\lanims.xr` (`UIEditLightAnim.cpp`).
//!
//! The list, the colour at the frame the pointer is on, a timeline drawn by hand with the keys
//! and the pointer, a key bar, and the animation's properties. Edits go through a `Store` with
//! undo; Save writes the file.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2};
use gz_app::{DocCx, Document, EditAction, OpenRequest};
use gz_editor::{Args, ArgsExt, CmdError, Editor, Origin, SmolStr};
use gz_fmt_xr::{LightAnim, LightAnims};
use gz_store::{Apply, State, Store, StoreOptions};
use gz_ui::dialog::{self, Answer, ConfirmSpec};
use gz_ui::grid::{color, integer, number, read_only};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Kind, PropFlags, PropertyGrid, Toolbar, ToolbarStyle, palette, state};
use serde_json::{Map, Value, json};

use super::list::{Empty, LibraryList, find, shown};
use super::{LibraryDoc, folder_text, pane, panes};
use crate::local::{local_checker, local_ends, local_heading};
use crate::paths::EditorDirs;

pub const KIND: &str = "light_animations";

/// The library as the store holds it.
#[derive(Clone, Default)]
pub struct LanimState {
    pub lib: Arc<LightAnims>,
    pub loaded: bool,
    /// Why the file could not be read.
    pub error: Option<String>,
}

/// What edits the library.
pub enum LanimCmd {
    Load { lib: LightAnims, error: Option<String> },
    Fps { name: SmolStr, fps: f32 },
    Frames { name: SmolStr, count: i32 },
    InsertKey { name: SmolStr, frame: i32, color: u32 },
    DeleteKey { name: SmolStr, frame: i32 },
    MoveKey { name: SmolStr, from: i32, to: i32 },
    /// A key's colour; edits of one key in a row are one undo step.
    SetKey { name: SmolStr, frame: i32, color: u32 },
    Saved,
}

fn item_mut<'a>(lib: &'a mut LightAnims, name: &str) -> Result<&'a mut LightAnim, CmdError> {
    lib.items.iter_mut().find(|i| i.name == name).ok_or_else(|| CmdError::not_found(format!("no light animation is named '{name}'")))
}

/// `CLAItem::Resize`: growing moves the key at the old end to the new end; shrinking drops the
/// keys past the new last frame.
pub fn resize(item: &mut LightAnim, count: i32) {
    let count = count.max(1);
    if count == item.frame_count {
        return;
    }
    if count > item.frame_count {
        let old = item.frame_count;
        item.frame_count = count;
        if let Some(c) = item.keys.remove(&old) {
            item.keys.insert(count, c);
        }
    } else {
        item.keys.retain(|f, _| *f <= count - 1);
        item.frame_count = count;
    }
}

/// `CLAItem::PrevKeyFrame`: the key before `frame`, else the first.
#[must_use]
pub fn prev_key(item: &LightAnim, frame: i32) -> i32 {
    item.keys.range(..frame).next_back().or_else(|| item.keys.iter().next()).map_or(0, |(f, _)| *f)
}

/// `CLAItem::NextKeyFrame`: the key after `frame`, else the last.
#[must_use]
pub fn next_key(item: &LightAnim, frame: i32) -> i32 {
    item.keys.range(frame + 1..).next().or_else(|| item.keys.iter().next_back()).map_or(0, |(f, _)| *f)
}

impl State for LanimState {
    type Cmd = LanimCmd;
    type Reply = ();

    fn apply(&mut self, cmd: LanimCmd, cx: &mut Apply) -> Result<(), CmdError> {
        match cmd {
            LanimCmd::Load { lib, error } => {
                self.lib = Arc::new(lib);
                self.loaded = true;
                self.error = error;
                cx.new_document();
            }
            LanimCmd::Saved => {
                cx.mark_saved();
                cx.unchanged();
            }
            LanimCmd::Fps { name, fps } => {
                let item = item_mut(Arc::make_mut(&mut self.lib), &name)?;
                item.fps = fps.clamp(0.1, 1000.0);
                cx.undoable_merged("FPS", format!("fps:{name}"));
            }
            LanimCmd::Frames { name, count } => {
                resize(item_mut(Arc::make_mut(&mut self.lib), &name)?, count.clamp(1, 100_000));
                cx.undoable("Frame Count");
            }
            LanimCmd::InsertKey { name, frame, color } => {
                let item = item_mut(Arc::make_mut(&mut self.lib), &name)?;
                if frame > item.frame_count || frame < 0 {
                    return Err(CmdError::invalid("the frame is past the animation's end"));
                }
                item.keys.insert(frame, color);
                cx.undoable("Add Key");
            }
            LanimCmd::DeleteKey { name, frame } => {
                let item = item_mut(Arc::make_mut(&mut self.lib), &name)?;
                // the first key stays: an animation keeps at least one
                if frame == 0 || item.keys.remove(&frame).is_none() {
                    cx.unchanged();
                    return Ok(());
                }
                cx.undoable("Delete Key");
            }
            LanimCmd::MoveKey { name, from, to } => {
                let item = item_mut(Arc::make_mut(&mut self.lib), &name)?;
                if from > item.frame_count || to > item.frame_count {
                    return Err(CmdError::invalid("the frame is past the animation's end"));
                }
                let Some(c) = item.keys.remove(&from) else {
                    cx.unchanged();
                    return Ok(());
                };
                item.keys.insert(to, c);
                cx.undoable("Move Key");
            }
            LanimCmd::SetKey { name, frame, color } => {
                let item = item_mut(Arc::make_mut(&mut self.lib), &name)?;
                let Some(c) = item.keys.get_mut(&frame) else {
                    return Err(CmdError::invalid("there is no key at this frame"));
                };
                *c = color;
                cx.undoable_merged("Key Color", format!("key:{name}:{frame}"));
            }
        }
        Ok(())
    }
}

/// `0xAARRGGBB` as a colour to paint; `alpha` off paints it opaque.
fn paint_color(argb: u32, alpha: bool) -> Color32 {
    let a = if alpha { (argb >> 24) as u8 } else { 255 };
    Color32::from_rgba_unmultiplied((argb >> 16) as u8, (argb >> 8) as u8, argb as u8, a)
}

/// The Light Animations tab.
pub struct LanimDoc {
    editor: Arc<Editor>,
    store: Store<LanimState>,
    file: PathBuf,
    list: LibraryList,
    pub selected: String,
    pub pointer: i32,
    /// When Play started, in the input's seconds, and the frame it started at.
    playing: Option<(f64, i32)>,
    pub render_alpha: bool,
    /// A frame count being dragged or typed: applied when the edit is let go of, so a drag
    /// through a smaller count drops no keys.
    pending_count: Option<i32>,
}

impl LanimDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>) -> Self {
        let store = Store::spawn("light animations", LanimState::default(), &editor.exec, StoreOptions::default());
        let file = EditorDirs::of(editor).lanims();
        let doc = Self { editor: editor.clone(), store, file, list: LibraryList::default(), selected: String::new(), pointer: 0, playing: None, render_alpha: false, pending_count: None };
        doc.reload_now();
        doc
    }

    /// Reads the file again (`LALib.Reload`) on the IO pool.
    fn reload_now(&self) {
        let (store, file, exec) = (self.store.clone(), self.file.clone(), self.editor.exec.clone());
        self.editor.exec.spawn(async move {
            let read = exec.io(move || std::fs::read(&file)).await;
            let (lib, error) = match read {
                Ok(bytes) => match LightAnims::read(&bytes) {
                    Ok(l) => (l, None),
                    Err(e) => (LightAnims::default(), Some(format!("lanims.xr could not be read: {e}"))),
                },
                // no file yet: an empty library, written by the first save
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (LightAnims::default(), None),
                Err(e) => (LightAnims::default(), Some(format!("lanims.xr could not be read: {e}"))),
            };
            let _ = store.send(LanimCmd::Load { lib, error }, Origin::Ui).await;
        });
    }

    fn reload(&mut self, ctx: &egui::Context) {
        if !self.store.is_dirty() {
            self.reload_now();
            return;
        }
        let spec = ConfirmSpec {
            id: "reload_light_animations".to_owned(),
            title: "Reload Light Animations".to_owned(),
            headline: "Read the light animation library from disk again?".to_owned(),
            items: vec!["Unsaved changes to the light animations".to_owned()],
            total: 0,
            consequence: "The changes made since the last save are lost.".to_owned(),
            action: "Reload".to_owned(),
            destructive: true,
        };
        let (store, file, exec) = (self.store.clone(), self.file.clone(), self.editor.exec.clone());
        let asker = if self.editor.flags.unattended { dialog::Asker::Unattended } else { dialog::Asker::Ui };
        let line = dialog::confirm(ctx, asker, spec, move |answer| {
            if answer != Answer::Yes {
                return;
            }
            let ex = exec.clone();
            exec.spawn(async move {
                let read = ex.io(move || std::fs::read(&file)).await;
                let lib = read.ok().and_then(|b| LightAnims::read(&b).ok()).unwrap_or_default();
                let _ = store.send(LanimCmd::Load { lib, error: None }, Origin::Ui).await;
            });
        });
        if let Some(line) = line {
            self.editor.log.plain(line);
        }
    }

    /// Writes the library (`LALib.Save`) on the IO pool, then marks it saved.
    fn save_now(&self) {
        let snap = self.store.snapshot();
        let bytes = snap.lib.write();
        let (store, file, exec, notices) = (self.store.clone(), self.file.clone(), self.editor.exec.clone(), self.editor.clone());
        self.editor.exec.spawn(async move {
            let shown = file.clone();
            match exec.io(move || gz_editor::fsx::write_atomic(&file, &bytes)).await {
                Ok(()) => {
                    let _ = store.send(LanimCmd::Saved, Origin::Ui).await;
                }
                Err(e) => notices.notices.error(format!("{} could not be written: {e}", shown.display())),
            }
        });
    }

    fn keys<'a>(lib: &'a LightAnims) -> Vec<&'a str> {
        lib.items.iter().map(|i| i.name.as_str()).collect()
    }

    fn current(&self, lib: &LightAnims) -> Option<LightAnim> {
        lib.items.iter().find(|i| i.name == self.selected).cloned()
    }

    fn clamp_pointer(&mut self, item: Option<&LightAnim>) {
        self.pointer = item.map_or(0, |i| self.pointer.clamp(0, (i.frame_count - 1).max(0)));
    }

    fn select(&mut self, name: &str) {
        self.selected = name.to_owned();
        self.playing = None;
        let snap = self.store.snapshot();
        let item = self.current(&snap.lib);
        self.clamp_pointer(item.as_ref());
    }

    fn post(&self, cmd: LanimCmd) {
        self.store.post(cmd, self.editor.ui_origin());
    }

    fn draw_preview(&mut self, ui: &mut Ui, item: &LightAnim) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        // Play runs the animation in real time: frames are asked for while it plays
        if let Some((start, from)) = self.playing {
            let fps = item.fps.max(1e-3);
            let len = (f64::from(item.frame_count) / f64::from(fps)).max(1e-3);
            let t = (ctx.input(|i| i.time) - start + f64::from(from) / f64::from(fps)).rem_euclid(len);
            self.pointer = ((t * f64::from(fps)).floor() as i32).clamp(0, (item.frame_count - 1).max(0));
            ctx.request_repaint();
        }
        let facts = format!("{} frames \u{b7} {} fps \u{b7} {:.2} s \u{b7} {} keys", item.frame_count, item.fps, item.frame_count as f32 / item.fps.max(1e-3), item.keys.len());
        local_heading(ui, &item.name, Clip::Start, &[(&facts, Clip::End)]);
        ui.add_space(8.0);

        // the colour at the frame the pointer is on: the light as it looks then
        let avail = ui.available_size();
        let swatch_h = (avail.y - (40.0 + 12.0 + 32.0 + 24.0)).clamp(48.0, 220.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(avail.x, swatch_h), Sense::hover());
        let argb = item.color_at(self.pointer.clamp(0, item.frame_count)).unwrap_or(0);
        if self.render_alpha {
            local_checker(ui.painter(), &p, rect.min, rect);
        }
        ui.painter().rect_filled(rect, CornerRadius::same(6), paint_color(argb, self.render_alpha));
        ui.painter().rect_stroke(rect, CornerRadius::same(6), Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        let tip = format!("Frame {}: #{:06X}, alpha {}{}", self.pointer, argb & 0xFF_FFFF, argb >> 24, if item.keys.contains_key(&self.pointer) { " (a key)" } else { "" });
        resp.on_hover_text(tip);
        ui.add_space(12.0);
        self.draw_timeline(ui, item, 40.0);
        self.draw_key_bar(ui, item);
    }

    fn draw_timeline(&mut self, ui: &mut Ui, item: &LightAnim, height: f32) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let count = item.frame_count.max(1);
        let w = ui.available_width().max(64.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, height), Sense::click_and_drag());
        let lane = 10.0;
        let strip_b = rect.max.y - lane;
        let under = |x: f32| (((x - rect.min.x) / w * count as f32).floor() as i32).clamp(0, count - 1);
        if let Some(pos) = resp.interact_pointer_pos() {
            if resp.double_clicked() {
                self.playing = None;
                self.pointer = under(pos.x);
                self.add_key(item);
            } else if resp.is_pointer_button_down_on() || resp.dragged() {
                self.playing = None;
                self.pointer = under(pos.x);
            }
        }
        let resp = resp.on_hover_text("Click or drag to pick a frame; double-click adds a key there.");
        let painter = ui.painter_at(rect);
        let strip = Rect::from_min_max(rect.min, pos2(rect.max.x, strip_b));
        // one strip per frame, or one per point column when the frames outnumber the points
        if self.render_alpha {
            local_checker(&painter, &p, strip.min, strip);
        }
        let cols = count.min((w as i32).max(1));
        for c in 0..cols {
            let frame = ((c as f32 / cols as f32) * count as f32) as i32;
            let argb = item.color_at(frame).unwrap_or(0);
            let x0 = rect.min.x + w * c as f32 / cols as f32;
            let x1 = rect.min.x + w * (c + 1) as f32 / cols as f32;
            painter.rect_filled(Rect::from_min_max(pos2(x0.floor(), rect.min.y), pos2(x1.floor() + 1.0, strip_b)), CornerRadius::ZERO, paint_color(argb, self.render_alpha));
        }
        painter.rect_filled(Rect::from_min_max(pos2(rect.min.x, strip_b), rect.max), CornerRadius::ZERO, p.inset);
        let step = w / count as f32;
        for f in item.keys.keys() {
            let x = (rect.min.x + (*f as f32 + 0.5) * step + 0.5).floor();
            let c = pos2(x, strip_b + lane * 0.5);
            let r = 3.5;
            painter.add(Shape::convex_polygon(vec![pos2(c.x, c.y - r), pos2(c.x + r, c.y), pos2(c.x, c.y + r), pos2(c.x - r, c.y)], p.text, Stroke::NONE));
        }
        let px = (rect.min.x + (self.pointer as f32 + 0.5) * step + 0.5).floor();
        painter.line_segment([Pos2::new(px, rect.min.y), Pos2::new(px, rect.max.y)], Stroke::new(2.0, p.accent_bright));
        painter.rect_stroke(rect, CornerRadius::ZERO, Stroke::new(1.0, p.border), egui::StrokeKind::Inside);
        let _ = resp;
        local_ends(ui, "0", &(count - 1).to_string());
    }

    fn add_key(&mut self, item: &LightAnim) {
        if item.keys.contains_key(&self.pointer) {
            return;
        }
        let color = item.color_at(self.pointer).unwrap_or(0);
        self.post(LanimCmd::InsertKey { name: item.name.clone(), frame: self.pointer, color });
    }

    fn draw_key_bar(&mut self, ui: &mut Ui, item: &LightAnim) {
        let on_key = item.keys.contains_key(&self.pointer);
        let now = ui.ctx().input(|i| i.time);
        let mut tb = Toolbar::begin(ui, "light_animation_keys", ToolbarStyle::Panel);
        tb.group(3);
        if tb.action("prev_key", Icon::ChevronLeft, "", "Go to the previous key", true, false, Kind::Ghost) {
            self.playing = None;
            self.pointer = prev_key(item, self.pointer);
        }
        let last = (item.frame_count - 1).max(0);
        let pointer = &mut self.pointer;
        let playing = &mut self.playing;
        tb.widget("frame", 88.0, |ui| {
            let r = ui.add(egui::DragValue::new(pointer).range(0..=last).speed(0.25).prefix("Frame "));
            if r.changed() {
                *playing = None;
            }
            r.on_hover_text("The frame the timeline points at; drag or type a number");
        });
        if tb.action("next_key", Icon::ChevronRight, "", "Go to the next key", true, false, Kind::Ghost) {
            self.playing = None;
            self.pointer = next_key(item, self.pointer);
        }
        tb.group(2);
        let add_tip = if on_key { "There is a key at this frame already" } else { "Add a key at this frame, with the colour the frame has now" };
        if tb.action("add_key", Icon::Plus, "Add Key", add_tip, !on_key, false, Kind::Ghost) {
            self.add_key(item);
        }
        let deletable = on_key && item.keys.len() > 1;
        let del_tip = if deletable {
            "Delete the key at this frame"
        } else if on_key {
            "An animation keeps at least one key"
        } else {
            "There is no key at this frame"
        };
        if tb.action("delete_key", Icon::Minus, "Delete Key", del_tip, deletable, false, Kind::Ghost) {
            self.post(LanimCmd::DeleteKey { name: item.name.clone(), frame: self.pointer });
        }
        tb.group(1);
        // a key moves one frame at a time, onto the next free frame
        let left = on_key && self.pointer > 0;
        let right = on_key && self.pointer < item.frame_count - 1;
        if tb.action("key_earlier", Icon::Back, "", if left { "Move this key to the nearest free frame before it" } else { "There is no key here to move" }, left, false, Kind::Ghost)
            && let Some(to) = (0..self.pointer).rev().find(|f| !item.keys.contains_key(f))
        {
            self.post(LanimCmd::MoveKey { name: item.name.clone(), from: self.pointer, to });
            self.pointer = to;
        }
        if tb.action("key_later", Icon::Forward, "", if right { "Move this key to the nearest free frame after it" } else { "There is no key here to move" }, right, false, Kind::Ghost)
            && let Some(to) = (self.pointer + 1..item.frame_count).find(|f| !item.keys.contains_key(f))
        {
            self.post(LanimCmd::MoveKey { name: item.name.clone(), from: self.pointer, to });
            self.pointer = to;
        }
        tb.spring();
        tb.group(4);
        let play = self.playing.is_some();
        if tb.action("play", if play { Icon::Stop } else { Icon::Play }, if play { "Stop" } else { "Play" }, if play { "Stop the animation" } else { "Play the animation in real time" }, true, play, Kind::Ghost) {
            self.playing = if play { None } else { Some((now, self.pointer)) };
        }
        tb.end();
    }

    fn draw_properties(&mut self, ui: &mut Ui, item: &LightAnim) {
        let name = item.name.clone();
        let mut fps = item.fps;
        let mut count = self.pending_count.unwrap_or(item.frame_count);
        let mut pending = self.pending_count;
        let key = item.keys.get(&self.pointer).copied();
        let pointer = self.pointer;
        let mut cmds = Vec::new();
        let mut seal = false;
        PropertyGrid::new("light_animation").show(ui, |g| {
            g.row("FPS", "Frames a second", PropFlags::NONE, 0, |ui| {
                let before = fps;
                let r = number(ui, &mut fps, 0.1, "fps");
                if fps != before {
                    cmds.push(LanimCmd::Fps { name: name.clone(), fps });
                }
                seal |= r.drag_stopped() || r.lost_focus();
            });
            g.row("Frame Count", "How many frames the animation runs", PropFlags::NONE, 0, |ui| {
                let before = count;
                let r = integer(ui, &mut count, 1.0, "");
                if count != before {
                    pending = Some(count.clamp(1, 100_000));
                }
                if !r.dragged() && !r.has_focus()
                    && let Some(n) = pending.take()
                    && n != item.frame_count
                {
                    cmds.push(LanimCmd::Frames { name: name.clone(), count: n });
                }
            });
            g.section("Current", true, None, |g| {
                g.row("Frame", "The frame the timeline points at", PropFlags::READ_ONLY, 0, |ui| read_only(ui, &pointer.to_string(), Clip::End));
                match key {
                    Some(argb) => {
                        g.row("Color", "The key's colour", PropFlags::NONE, 0, |ui| {
                            let mut c = paint_color(argb, false);
                            let r = color(ui, &mut c);
                            if r.changed() {
                                let rgb = (u32::from(c.r()) << 16) | (u32::from(c.g()) << 8) | u32::from(c.b());
                                cmds.push(LanimCmd::SetKey { name: name.clone(), frame: pointer, color: (argb & 0xFF00_0000) | rgb });
                            }
                        });
                        g.row("Alpha", "The key's alpha, 0 to 255", PropFlags::NONE, 0, |ui| {
                            let mut a = (argb >> 24) as i32;
                            let r = integer(ui, &mut a, 1.0, "");
                            if a != (argb >> 24) as i32 {
                                let a = a.clamp(0, 255) as u32;
                                cmds.push(LanimCmd::SetKey { name: name.clone(), frame: pointer, color: (a << 24) | (argb & 0xFF_FFFF) });
                            }
                            seal |= r.drag_stopped() || r.lost_focus();
                        });
                    }
                    None => {
                        g.row("Key", "", PropFlags::READ_ONLY, 0, |ui| read_only(ui, "none at this frame", Clip::End));
                    }
                }
            });
        });
        self.pending_count = pending;
        for c in cmds {
            self.post(c);
        }
        if seal {
            self.store.seal();
        }
    }

    /// The fields of `library` for this tab (`UIEditLightAnim::Mcp`).
    fn state(&self) -> Map<String, Value> {
        let snap = self.store.snapshot();
        let keys = Self::keys(&snap.lib);
        let rows = shown(&keys, &|_| true, &self.list.search);
        let mut out = Map::new();
        out.insert("items".into(), json!(keys.len()));
        out.insert("shown".into(), json!(rows.len()));
        out.insert("first".into(), json!(rows.first().and_then(|&i| keys.get(i)).copied().unwrap_or("")));
        out.insert("dirty".into(), json!(self.store.is_dirty()));
        let item = self.current(&snap.lib);
        out.insert("selected".into(), json!(item.as_ref().map_or("", |i| i.name.as_str())));
        if let Some(i) = &item {
            out.insert("frame".into(), json!(self.pointer));
            out.insert("frames".into(), json!(i.frame_count));
            out.insert("fps".into(), json!(i.fps));
            out.insert("keys".into(), json!(i.keys.len()));
            out.insert("on_key".into(), json!(i.keys.contains_key(&self.pointer)));
        }
        out.insert("alpha".into(), json!(self.render_alpha));
        out.insert("file".into(), json!(crate::paths::json_path(&self.file)));
        out
    }

    /// Waits until the file was read: what `library` answers about is the library.
    pub async fn loaded(store: &Store<LanimState>) {
        let mut rx = store.subscribe();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        while !store.snapshot().loaded {
            if tokio::time::timeout_at(deadline, rx.changed()).await.is_err() {
                return;
            }
        }
    }

    #[must_use]
    pub fn store(&self) -> Store<LanimState> {
        self.store.clone()
    }
}

impl LibraryDoc for LanimDoc {
    fn mcp(&mut self, action: &str, args: &Args) -> Result<Map<String, Value>, String> {
        if let Some(s) = args.str("search") {
            self.list.search = s.to_owned();
        }
        if action == "select" {
            let item = args.str("item").unwrap_or_default().to_owned();
            let snap = self.store.snapshot();
            let keys = Self::keys(&snap.lib);
            let Some(at) = find(&keys, &item) else { return Err(format!("no light animation is named or matches '{item}'")) };
            let name = keys.get(at).copied().unwrap_or_default().to_owned();
            self.select(&name);
            if let Some(frame) = args.i64("frame").filter(|f| *f >= 0) {
                self.pointer = i32::try_from(frame).unwrap_or(i32::MAX);
                let item = self.current(&snap.lib);
                self.clamp_pointer(item.as_ref());
            }
        }
        Ok(self.state())
    }
}

impl Document for LanimDoc {
    fn id(&self) -> &str {
        KIND
    }
    fn kind(&self) -> &'static str {
        KIND
    }
    fn label(&self) -> String {
        "Light Animations".to_owned()
    }
    fn icon(&self) -> Icon {
        Icon::Light
    }
    fn tooltip(&self) -> Option<String> {
        Some("The light animation library (lanims.xr)".to_owned())
    }
    fn dirty(&self) -> bool {
        self.store.is_dirty()
    }
    fn padded(&self) -> bool {
        false
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        let dirty = self.store.is_dirty();
        tb.group(1);
        if tb.action("save", Icon::Save, "Save", if dirty { "Save the light animation library (lanims.xr)" } else { "No unsaved changes" }, dirty, false, Kind::Ghost) {
            self.save_now();
        }
        if tb.action("reload", Icon::Reload, "Reload", "Read the light animation library from disk again", true, false, Kind::Ghost) {
            self.reload(cx.ctx);
        }
        tb.group(0);
        if tb.action("alpha", Icon::Eye, "Alpha", "Show the colours' alpha over a checkerboard", true, self.render_alpha, Kind::Ghost) {
            self.render_alpha = !self.render_alpha;
        }
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        let snap = self.store.snapshot();
        let [list_r, preview_r, details_r] = panes(ui, KIND);
        let keys = Self::keys(&snap.lib);
        let rows = shown(&keys, &|_| true, &self.list.search);
        // only an empty list says where the library is read from
        let empty_text = if snap.loaded && keys.is_empty() {
            format!("lanims.xr in {} holds no light animation.", folder_text(&self.editor, &EditorDirs::of(&self.editor).gamedata))
        } else {
            String::new()
        };
        let empty = Empty { title: if snap.loaded { "No light animations" } else { "Reading the library..." }, text: &empty_text, action: None };
        let selected = self.selected.clone();
        let out = pane(ui, list_r, "light_animations.list.pane", false, false, |ui| self.list.draw(ui, "light_animations", &keys, &rows, &selected, &empty));
        if let Some(i) = out.picked
            && let Some(name) = keys.get(i)
        {
            let name = (*name).to_owned();
            self.select(&name);
        }
        let item = self.current(&snap.lib);
        if let Some(e) = &snap.error {
            pane(ui, preview_r, "light_animations.preview", true, false, |ui| {
                state::empty_state(ui, Icon::Warning, "The library cannot be read", e, None);
            });
        } else {
            pane(ui, preview_r, "light_animations.preview", true, false, |ui| match &item {
                Some(item) => self.draw_preview(ui, item),
                None => {
                    state::empty_state(ui, Icon::Light, "No light animation selected", "Pick one in the list to see its colours and edit its keys.", None);
                }
            });
        }
        pane(ui, details_r, "light_animations.details", false, true, |ui| match &item {
            Some(item) => self.draw_properties(ui, item),
            None => {
                state::empty_state(ui, Icon::Settings, "No properties", "Pick a light animation to edit it.", None);
            }
        });
        ui.allocate_rect(ui.max_rect(), Sense::hover());
    }

    fn status(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        let snap = self.store.snapshot();
        let mut line = format!("{} light animations", snap.lib.items.len());
        if self.store.is_dirty() {
            line.push_str(" \u{b7} unsaved changes");
        }
        let p = palette(ui.ctx());
        text::label(ui, &line, Font::Small, p.text_secondary, Clip::End);
    }

    fn edit(&mut self, _cx: &mut DocCx<'_>, action: EditAction, run: bool) -> bool {
        let h = self.store.history();
        let there = match action {
            EditAction::Undo => !h.undo.is_empty(),
            EditAction::Redo => !h.redo.is_empty(),
            _ => return false,
        };
        if run && there {
            let store = self.store.clone();
            self.editor.exec.spawn(async move {
                match action {
                    EditAction::Undo => store.undo().await,
                    _ => store.redo().await,
                };
            });
        }
        there
    }

    fn save(&mut self, _cx: &mut DocCx<'_>) -> Result<(), String> {
        self.save_now();
        Ok(())
    }

    fn discard(&mut self, _cx: &mut DocCx<'_>) {
        self.reload_now();
    }

    fn on_close(&mut self, _cx: &mut DocCx<'_>) {
        self.playing = None;
    }

    fn mcp_state(&self) -> Value {
        Value::Object(self.state())
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// The `light_animations` factory.
pub fn factory(editor: &Arc<Editor>, _req: &OpenRequest) -> Result<Box<dyn Document>, String> {
    Ok(Box::new(LanimDoc::new(editor)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn item() -> LightAnim {
        LightAnim { name: "flicker".into(), fps: 15.0, frame_count: 20, keys: BTreeMap::from([(0, 0xff00_0000), (10, 0xffff_ffff), (20, 0xff10_1010)]) }
    }

    #[test]
    fn resize_moves_the_end_key_and_drops_past_it() {
        let mut i = item();
        resize(&mut i, 30);
        assert_eq!(i.frame_count, 30);
        assert!(i.keys.contains_key(&30) && !i.keys.contains_key(&20));
        resize(&mut i, 10);
        assert_eq!(i.keys.keys().copied().collect::<Vec<_>>(), vec![0]);
    }

    #[test]
    fn keys_step() {
        let i = item();
        assert_eq!(prev_key(&i, 10), 0);
        assert_eq!(prev_key(&i, 0), 0);
        assert_eq!(next_key(&i, 10), 20);
        assert_eq!(next_key(&i, 20), 20);
    }
}
