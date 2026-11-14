//! Light Animations: the `light_animations` tab over `gamedata\lanims.xr`.
//!
//! The list, the colour at the frame the pointer is on, a timeline drawn by hand with the keys
//! and the pointer, a key bar, and the animation's properties. Edits go through a `Store` with
//! undo; Save writes the file.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2, pos2};
use gz_app::{DocCx, Document, EditAction, OpenRequest};
use gz_editor::{Args, CmdError, Editor, Origin, SmolStr};
use gz_fmt_xr::{LightAnim, LightAnims};
use gz_store::{Apply, State, Store};
use gz_ui::dialog::{self, Answer, ConfirmSpec};
use gz_ui::grid::{color, integer, number, read_only};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, Kind, PropFlags, PropertyGrid, Toolbar, ToolbarStyle, palette, state};
use serde_json::{Map, Value, json};

use super::keys;
use super::list::{Empty, LibraryList, find, shown};
use super::stores::{LanimFacts, LibraryFacts, LibraryStores};
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
    Load {
        lib: LightAnims,
        error: Option<String>,
    },
    Fps {
        name: SmolStr,
        fps: f32,
    },
    Frames {
        name: SmolStr,
        count: i32,
    },
    InsertKey {
        name: SmolStr,
        frame: i32,
        color: u32,
    },
    DeleteKey {
        name: SmolStr,
        frame: i32,
    },
    MoveKey {
        name: SmolStr,
        from: i32,
        to: i32,
    },
    /// A key's colour; edits of one key in a row are one undo step.
    SetKey {
        name: SmolStr,
        frame: i32,
        color: u32,
    },
    Saved,
}

fn item_mut<'a>(lib: &'a mut LightAnims, name: &str) -> Result<&'a mut LightAnim, CmdError> {
    lib.items
        .iter_mut()
        .find(|i| i.name == name)
        .ok_or_else(|| CmdError::not_found(format!("no light animation is named '{name}'")))
}

/// Growing moves the key at the old end to the new end; shrinking drops the
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
        item.keys.retain(|f, _| *f < count);
        item.frame_count = count;
    }
}

/// The key before `frame`, else the first.
#[must_use]
pub fn prev_key(item: &LightAnim, frame: i32) -> i32 {
    item.keys.range(..frame).next_back().or_else(|| item.keys.iter().next()).map_or(0, |(f, _)| *f)
}

/// The key after `frame`, else the last.
#[must_use]
pub fn next_key(item: &LightAnim, frame: i32) -> i32 {
    item.keys.range(frame + 1..).next().or_else(|| item.keys.iter().next_back()).map_or(0, |(f, _)| *f)
}

/// A key added at `frame`.
pub fn insert_key(item: &mut LightAnim, frame: i32, color: u32) -> Result<(), CmdError> {
    if frame > item.frame_count || frame < 0 {
        return Err(CmdError::invalid("the frame is past the animation's end"));
    }
    item.keys.insert(frame, color);
    Ok(())
}

/// The key at `frame` taken away; `false` when nothing changed (the first key stays, an
/// animation keeps at least one; no key was there).
pub fn delete_key(item: &mut LightAnim, frame: i32) -> bool {
    frame != 0 && item.keys.remove(&frame).is_some()
}

/// The key at `from` moved to `to`; `false` when there is no key at `from`.
pub fn move_key(item: &mut LightAnim, from: i32, to: i32) -> Result<bool, CmdError> {
    if from > item.frame_count || to > item.frame_count {
        return Err(CmdError::invalid("the frame is past the animation's end"));
    }
    let Some(c) = item.keys.remove(&from) else { return Ok(false) };
    item.keys.insert(to, c);
    Ok(true)
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
                insert_key(item_mut(Arc::make_mut(&mut self.lib), &name)?, frame, color)?;
                cx.undoable("Add Key");
            }
            LanimCmd::DeleteKey { name, frame } => {
                if !delete_key(item_mut(Arc::make_mut(&mut self.lib), &name)?, frame) {
                    cx.unchanged();
                    return Ok(());
                }
                cx.undoable("Delete Key");
            }
            LanimCmd::MoveKey { name, from, to } => {
                if !move_key(item_mut(Arc::make_mut(&mut self.lib), &name)?, from, to)? {
                    cx.unchanged();
                    return Ok(());
                }
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
        let store = LibraryStores::of(editor).lanim.clone();
        let file = EditorDirs::of(editor).lanims();
        let doc = Self {
            editor: editor.clone(),
            store,
            file,
            list: LibraryList::default(),
            selected: String::new(),
            pointer: 0,
            playing: None,
            render_alpha: false,
            pending_count: None,
        };
        // changes made before the tab opened (a command's) stay; a clean library is read again
        if !doc.store.is_dirty() {
            doc.reload_now();
        }
        doc
    }

    /// Reads the file again (`LALib.Reload`) on the IO pool.
    fn reload_now(&self) {
        let editor = self.editor.clone();
        self.editor.exec.spawn(async move { LibraryStores::reload_lanims(&editor).await });
    }

    /// Writes the library (`LALib.Save`) on the IO pool, then marks it saved.
    fn save_now(&self) {
        let editor = self.editor.clone();
        self.editor.exec.spawn(async move {
            if let Err(e) = save_lanims(&editor).await {
                editor.notices.error(e);
            }
        });
    }

    /// Plays the selected animation in real time, or stops it.
    pub fn play(&mut self, play: bool, now: f64) -> Result<(), String> {
        if !play {
            self.playing = None;
            return Ok(());
        }
        if self.selected.is_empty() {
            return Err("Pick a light animation first.".to_owned());
        }
        self.playing = Some((now, self.pointer));
        Ok(())
    }

    /// The frame the timeline points at.
    pub fn set_pointer(&mut self, frame: i32) {
        self.playing = None;
        self.pointer = frame.max(0);
        let snap = self.store.snapshot();
        let item = self.current(&snap.lib);
        self.clamp_pointer(item.as_ref());
    }

    fn keys(lib: &LightAnims) -> Vec<&str> {
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

    fn draw_preview(&mut self, ui: &mut Ui, item: &LightAnim, cx: &mut DocCx<'_>) {
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
        let facts = format!(
            "{} frames \u{b7} {} fps \u{b7} {:.2} s \u{b7} {} keys",
            item.frame_count,
            item.fps,
            item.frame_count as f32 / item.fps.max(1e-3),
            item.keys.len()
        );
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
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(6),
            Stroke::new(1.0, p.border),
            egui::StrokeKind::Inside,
        );
        let tip = format!(
            "Frame {}: #{:06X}, alpha {}{}",
            self.pointer,
            argb & 0xFF_FFFF,
            argb >> 24,
            if item.keys.contains_key(&self.pointer) { " (a key)" } else { "" }
        );
        resp.on_hover_text(tip);
        ui.add_space(12.0);
        self.draw_timeline(ui, item, 40.0);
        self.draw_key_bar(ui, item, cx);
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
            painter.rect_filled(
                Rect::from_min_max(pos2(x0.floor(), rect.min.y), pos2(x1.floor() + 1.0, strip_b)),
                CornerRadius::ZERO,
                paint_color(argb, self.render_alpha),
            );
        }
        painter.rect_filled(
            Rect::from_min_max(pos2(rect.min.x, strip_b), rect.max),
            CornerRadius::ZERO,
            p.inset,
        );
        let step = w / count as f32;
        for f in item.keys.keys() {
            let x = (rect.min.x + (*f as f32 + 0.5) * step + 0.5).floor();
            let c = pos2(x, strip_b + lane * 0.5);
            let r = 3.5;
            painter.add(Shape::convex_polygon(
                vec![pos2(c.x, c.y - r), pos2(c.x + r, c.y), pos2(c.x, c.y + r), pos2(c.x - r, c.y)],
                p.text,
                Stroke::NONE,
            ));
        }
        let px = (rect.min.x + (self.pointer as f32 + 0.5) * step + 0.5).floor();
        painter.line_segment(
            [Pos2::new(px, rect.min.y), Pos2::new(px, rect.max.y)],
            Stroke::new(2.0, p.accent_bright),
        );
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

    fn draw_key_bar(&mut self, ui: &mut Ui, item: &LightAnim, cx: &mut DocCx<'_>) {
        let on_key = item.keys.contains_key(&self.pointer);
        let free_before = (0..self.pointer).rev().find(|f| !item.keys.contains_key(f));
        let free_after = (self.pointer + 1..item.frame_count).find(|f| !item.keys.contains_key(f));
        LibraryFacts::of(&self.editor).set_lanim(LanimFacts {
            selected: true,
            pointer: self.pointer,
            frames: item.frame_count,
            on_key,
            keys: item.keys.len(),
            playing: self.playing.is_some(),
            alpha: self.render_alpha,
            color: item.color_at(self.pointer).unwrap_or(0),
            free_before: free_before.filter(|_| on_key),
            free_after: free_after.filter(|_| on_key),
        });
        let mut tb = Toolbar::begin(ui, "light_animation_keys", ToolbarStyle::Panel);
        tb.group(3);
        gz_app::toolbar_button(&mut tb, cx, "library.previous_key", Kind::Ghost, Some(""));
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
        gz_app::toolbar_button(&mut tb, cx, "library.next_key", Kind::Ghost, Some(""));
        tb.group(2);
        gz_app::toolbar_button(&mut tb, cx, "library.add_key", Kind::Ghost, None);
        gz_app::toolbar_button(&mut tb, cx, "library.delete_key", Kind::Ghost, None);
        tb.group(1);
        gz_app::toolbar_button(&mut tb, cx, "library.key_earlier", Kind::Ghost, Some(""));
        gz_app::toolbar_button(&mut tb, cx, "library.key_later", Kind::Ghost, Some(""));
        tb.spring();
        tb.group(4);
        let play = self.playing.is_some();
        gz_app::toolbar_button(
            &mut tb,
            cx,
            if play { "library.stop_animation" } else { "library.play_animation" },
            Kind::Ghost,
            None,
        );
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
                if !r.dragged()
                    && !r.has_focus()
                    && let Some(n) = pending.take()
                    && n != item.frame_count
                {
                    cmds.push(LanimCmd::Frames { name: name.clone(), count: n });
                }
            });
            g.section("Current", true, None, |g| {
                g.row("Frame", "The frame the timeline points at", PropFlags::READ_ONLY, 0, |ui| {
                    read_only(ui, &pointer.to_string(), Clip::End)
                });
                match key {
                    Some(argb) => {
                        g.row("Color", "The key's colour", PropFlags::NONE, 0, |ui| {
                            let mut c = paint_color(argb, false);
                            let r = color(ui, &mut c);
                            if r.changed() {
                                let rgb =
                                    (u32::from(c.r()) << 16) | (u32::from(c.g()) << 8) | u32::from(c.b());
                                cmds.push(LanimCmd::SetKey {
                                    name: name.clone(),
                                    frame: pointer,
                                    color: (argb & 0xFF00_0000) | rgb,
                                });
                            }
                        });
                        g.row("Alpha", "The key's alpha, 0 to 255", PropFlags::NONE, 0, |ui| {
                            let mut a = (argb >> 24) as i32;
                            let r = integer(ui, &mut a, 1.0, "");
                            if a != (argb >> 24) as i32 {
                                let a = a.clamp(0, 255) as u32;
                                cmds.push(LanimCmd::SetKey {
                                    name: name.clone(),
                                    frame: pointer,
                                    color: (a << 24) | (argb & 0xFF_FFFF),
                                });
                            }
                            seal |= r.drag_stopped() || r.lost_focus();
                        });
                    }
                    None => {
                        g.row("Key", "", PropFlags::READ_ONLY, 0, |ui| {
                            read_only(ui, "none at this frame", Clip::End)
                        });
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

    /// The fields of `library` for this tab.
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
        out.insert("playing".into(), json!(self.playing.is_some()));
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
        if let Some(s) = args.get("search").and_then(Value::as_str) {
            self.list.search = s.to_owned();
        }
        if let Some(on) = args.get("alpha").and_then(Value::as_bool) {
            self.render_alpha = on;
        }
        if action == "select" {
            let item = args.get("item").and_then(Value::as_str).unwrap_or_default().to_owned();
            let snap = self.store.snapshot();
            let keys = Self::keys(&snap.lib);
            let Some(at) = find(&keys, &item) else {
                return Err(format!("no light animation is named or matches '{item}'"));
            };
            let name = keys.get(at).copied().unwrap_or_default().to_owned();
            self.select(&name);
            if let Some(frame) = args.get("frame").and_then(Value::as_i64).filter(|f| *f >= 0) {
                self.pointer = i32::try_from(frame).unwrap_or(i32::MAX);
                let item = self.current(&snap.lib);
                self.clamp_pointer(item.as_ref());
            }
        }
        // the previous or the next key of the selected animation
        if let Some(step) = args.get("key").and_then(Value::as_str) {
            let snap = self.store.snapshot();
            let Some(item) = self.current(&snap.lib) else {
                return Err("Pick a light animation first.".to_owned());
            };
            self.playing = None;
            self.pointer = match step {
                "previous" => prev_key(&item, self.pointer),
                _ => next_key(&item, self.pointer),
            };
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
        if tb.action(
            "save",
            Icon::Save,
            "Save",
            if dirty { "Save the light animation library (lanims.xr)" } else { "No unsaved changes" },
            dirty,
            false,
            Kind::Ghost,
        ) {
            cx.requests.action("file.save");
        }
        gz_app::toolbar_button(tb, cx, "library.reload_lanims", Kind::Ghost, None);
        tb.group(0);
        gz_app::toolbar_button(tb, cx, "library.alpha", Kind::Ghost, None);
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let snap = self.store.snapshot();
        let keys = Self::keys(&snap.lib);
        // a library with nothing in it (or that cannot be read) says so once, over the whole tab
        if snap.loaded && keys.is_empty() {
            match &snap.error {
                Some(e) => state::error_state(ui, "The library cannot be read", e, None),
                None => {
                    let text_ = format!(
                        "lanims.xr in {} holds no light animation.",
                        folder_text(&self.editor, &EditorDirs::of(&self.editor).gamedata)
                    );
                    state::empty_state(ui, Icon::Light, "No light animations", &text_, None)
                }
            };
            return;
        }
        let [list_r, preview_r, details_r] = panes(ui, KIND);
        let rows = shown(&keys, &|_| true, &self.list.search);
        // only an empty list says where the library is read from
        let empty_text = if snap.loaded && keys.is_empty() {
            format!(
                "lanims.xr in {} holds no light animation.",
                folder_text(&self.editor, &EditorDirs::of(&self.editor).gamedata)
            )
        } else {
            String::new()
        };
        let empty = Empty {
            title: if snap.loaded { "No light animations" } else { "Reading the library..." },
            text: &empty_text,
            action: None,
        };
        let selected = self.selected.clone();
        let out = pane(ui, list_r, "light_animations.list.pane", false, false, |ui| {
            self.list.draw(ui, "light_animations", &keys, &rows, &selected, &empty)
        });
        if let Some(i) = out.picked
            && let Some(name) = keys.get(i)
        {
            let name = (*name).to_owned();
            self.select(&name);
        }
        let item = self.current(&snap.lib);
        if item.is_none() {
            LibraryFacts::of(&self.editor)
                .set_lanim(LanimFacts { alpha: self.render_alpha, ..LanimFacts::default() });
        }
        if let Some(e) = &snap.error {
            pane(ui, preview_r, "light_animations.preview", true, false, |ui| {
                state::error_state(ui, "The library cannot be read", e, None);
            });
        } else {
            pane(ui, preview_r, "light_animations.preview", true, false, |ui| match &item {
                Some(item) => self.draw_preview(ui, item, cx),
                None => {
                    state::empty_state(
                        ui,
                        Icon::Light,
                        "No light animation selected",
                        "Pick one in the list to see its colours and edit its keys.",
                        None,
                    );
                }
            });
        }
        // nothing picked: the preview says so, the details stay blank
        pane(ui, details_r, "light_animations.details", false, true, |ui| {
            if let Some(item) = &item {
                self.draw_properties(ui, item);
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

/// Reload's answer in the window: a library with unsaved changes asks before they go, and Yes
/// runs the command again with them dropped.
pub fn reload_reply(r: &gz_app::ReplyCx<'_>) {
    let code = r.reply.get("code").and_then(Value::as_str);
    if code != Some("unsaved") {
        if r.reply.get("ok").and_then(Value::as_bool) != Some(true) {
            let text = r.reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.");
            r.editor.notices.error(text.to_owned());
        }
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
    let editor = r.editor.clone();
    let asker = if editor.flags.unattended { dialog::Asker::Unattended } else { dialog::Asker::Ui };
    let ed = editor.clone();
    let line = dialog::confirm(r.ctx, asker, spec, move |answer| {
        if answer != Answer::Yes {
            return;
        }
        let mut a = Args::new();
        a.insert("kind".into(), json!(KIND));
        a.insert("discard".into(), json!(true));
        ed.post("library.reload", a);
    });
    if let Some(line) = line {
        editor.log.finding(&line);
    }
}

/// Writes the library as the store holds it, then marks it saved.
pub async fn save_lanims(editor: &Arc<Editor>) -> Result<(), String> {
    let stores = LibraryStores::of(editor);
    let bytes = stores.lanim.snapshot().lib.write();
    let file = EditorDirs::of(editor).lanims();
    let shown = file.clone();
    editor
        .exec
        .io(move || gz_editor::fsx::write_atomic(&file, &bytes))
        .await
        .map_err(|e| format!("{} could not be written: {e}", shown.display()))?;
    let _ = stores.lanim.send(LanimCmd::Saved, Origin::Ui).await;
    Ok(())
}

/// The keys `library.set` takes of a light animation.
pub const LANIM_KEYS: [(&str, &str); 4] = [
    ("fps", "frames a second, 0.1 to 1000"),
    (
        "frames",
        "how many frames it runs, 1 to 100000; growing moves the last key to the new end, shrinking drops the keys past it",
    ),
    (
        "keys",
        "an object of frame to colour (#RRGGBB or #AARRGGBB): a key added or recoloured; null deletes the key there",
    ),
    (
        "move_key",
        "an object {from, to}: the key at from moves to the free frame to; follow=true takes the timeline's pointer along",
    ),
];

/// What a set of a light animation did: the changes, and the frame the pointer follows to.
pub struct LanimSet {
    pub changed: Vec<Value>,
    pub follow: Option<i32>,
}

/// Applies `set` to the light animation `name`, all or none: checked on a copy first, then sent
/// to the store as the edits a person's are.
pub async fn set_lanim(
    editor: &Arc<Editor>,
    name: &str,
    set: &Map<String, Value>,
) -> Result<LanimSet, CmdError> {
    let names: Vec<&str> = LANIM_KEYS.iter().map(|(k, _)| *k).collect();
    if let Some(k) = set.keys().find(|k| !names.contains(&k.as_str())) {
        return Err(CmdError::invalid(keys::unknown_key("a light animation", k, &names)));
    }
    let snap = LibraryStores::lanims_ready(editor).await;
    if let Some(e) = &snap.error {
        return Err(CmdError::new("failed", e.clone()));
    }
    let all: Vec<&str> = snap.lib.items.iter().map(|i| i.name.as_str()).collect();
    let name = keys::exact("light animation", &all, name).map_err(CmdError::not_found)?.to_owned();
    let mut work =
        snap.lib.items.iter().find(|i| i.name == name).cloned().ok_or_else(|| CmdError::not_found(&name))?;
    let (mut ops, mut changed, mut follow) = (Vec::new(), Vec::new(), None);
    let bad = |text: String| CmdError::invalid(text);
    let n: SmolStr = name.as_str().into();
    if let Some(v) = set.get("fps") {
        let fps =
            v.as_f64().filter(|x| x.is_finite()).ok_or_else(|| bad("fps takes a number".into()))? as f32;
        let fps = fps.clamp(0.1, 1000.0);
        changed.push(json!({ "key": "fps", "from": work.fps, "to": fps }));
        work.fps = fps;
        ops.push(LanimCmd::Fps { name: n.clone(), fps });
    }
    if let Some(v) = set.get("frames") {
        let count = v
            .as_i64()
            .filter(|c| (1..=100_000).contains(c))
            .ok_or_else(|| bad("frames takes a whole number, 1 to 100000".into()))?;
        let count = i32::try_from(count).unwrap_or(1);
        changed.push(json!({ "key": "frames", "from": work.frame_count, "to": count }));
        resize(&mut work, count);
        ops.push(LanimCmd::Frames { name: n.clone(), count });
    }
    if let Some(v) = set.get("keys") {
        let Some(map) = v.as_object() else {
            return Err(bad("keys takes an object of frame to colour".into()));
        };
        let mut at: Vec<(i32, &Value)> = Vec::new();
        for (k, val) in map {
            let frame =
                k.trim().parse::<i32>().map_err(|_| bad(format!("keys: '{k}' is no frame number")))?;
            at.push((frame, val));
        }
        at.sort_by_key(|(f, _)| *f);
        for (frame, val) in at {
            if val.is_null() {
                if frame == 0 {
                    return Err(bad("the first key stays: an animation keeps at least one".into()));
                }
                let was = work.keys.get(&frame).copied();
                if !delete_key(&mut work, frame) {
                    return Err(bad(format!("there is no key at frame {frame}")));
                }
                changed.push(
                    json!({ "key": "keys", "frame": frame, "from": was.map(keys::color_text), "to": null }),
                );
                ops.push(LanimCmd::DeleteKey { name: n.clone(), frame });
                continue;
            }
            let color = keys::color("keys", val).map_err(bad)?;
            let was = work.keys.get(&frame).copied();
            if was == Some(color) {
                continue;
            }
            if was.is_some() {
                work.keys.insert(frame, color);
                ops.push(LanimCmd::SetKey { name: n.clone(), frame, color });
            } else {
                insert_key(&mut work, frame, color)?;
                ops.push(LanimCmd::InsertKey { name: n.clone(), frame, color });
            }
            changed.push(json!({ "key": "keys", "frame": frame, "from": was.map(keys::color_text), "to": keys::color_text(color) }));
        }
    }
    if let Some(v) = set.get("move_key") {
        let Some(m) = v.as_object() else { return Err(bad("move_key takes {from, to}".into())) };
        for k in m.keys() {
            if !matches!(k.as_str(), "from" | "to" | "follow") {
                return Err(bad(keys::unknown_key("move_key", k, &["from", "to", "follow"])));
            }
        }
        let frame_of = |k: &str| -> Result<i32, CmdError> {
            m.get(k)
                .and_then(Value::as_i64)
                .and_then(|f| i32::try_from(f).ok())
                .ok_or_else(|| bad(format!("move_key takes {k} as a frame number")))
        };
        let (from, to) = (frame_of("from")?, frame_of("to")?);
        if !work.keys.contains_key(&from) {
            return Err(bad(format!("there is no key at frame {from}")));
        }
        if work.keys.contains_key(&to) {
            return Err(bad(format!("frame {to} has a key already")));
        }
        move_key(&mut work, from, to)?;
        if m.get("follow").and_then(Value::as_bool) == Some(true) {
            follow = Some(to);
        }
        changed.push(json!({ "key": "move_key", "from": from, "to": to }));
        ops.push(LanimCmd::MoveKey { name: n.clone(), from, to });
    }
    for op in ops {
        LibraryStores::of(editor).lanim.send(op, Origin::Mcp).await?;
    }
    Ok(LanimSet { changed, follow })
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
        LightAnim {
            name: "flicker".into(),
            fps: 15.0,
            frame_count: 20,
            keys: BTreeMap::from([(0, 0xff00_0000), (10, 0xffff_ffff), (20, 0xff10_1010)]),
        }
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
