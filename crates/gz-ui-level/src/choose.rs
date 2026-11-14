//! The chooser `Select <what>###dlg.choose` (L, most of the window's height): a ranked search
//! over a list, rows with a picture where the list has pictures (models, library objects,
//! textures), a preview of the highlighted item (its picture, or Play for a sound), the item
//! picked. What comes from a list is picked, never typed: an empty list says what it lacks.
//!
//! The list's order for a search is worked out on the CPU pool (the kit's ranking: whole-word
//! prefixes first, then letters in order), only the rows in view are laid out, and the pictures
//! are made on the pools: opening a chooser of 5000 rows or typing in it never holds a frame.
//!
//! Who asked decides what a pick does ([`Purpose`]); the dialog only answers what was picked.

use std::sync::Arc;

use egui::{Key, Modifiers, Rect, Sense, Ui, Vec2, pos2};
use gz_base::{Rev, SmolStr};
use gz_editor::{Editor, Value, json};
use gz_query::Query;
use gz_ui::text::{self, Clip};
use gz_ui::{Dialog, DialogSize, Font, Icon, Kind, SearchField, Tone, palette, state};

use crate::choose_pics::{PicKind, Pics};
use crate::lists::{ListKey, Lists};

/// What the pick is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Purpose {
    /// A choose row of Details (target 0) or Level Settings (target 1).
    Property { target: usize, key: String },
    /// Library objects the Foliage panel adds as meshes (several).
    DetailObjects,
    /// The objects a colour index scatters (several), by the index's position.
    ColorIndexObjects(usize),
    /// A tool command takes the pick as an argument (`scene.ai_map action=set ignore_materials=`).
    ToolCommand { cmd: &'static str, action: &'static str, arg: &'static str },
}

/// The order of the items for a search, made off the UI thread.
#[derive(Clone)]
struct Order(Query<(), Vec<u32>>);

impl std::fmt::Debug for Order {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Order")
    }
}

/// The items a scene gives (its way paths, its story ids), read off the UI thread.
#[derive(Clone)]
struct SceneItems(Query<(), Vec<String>>);

impl std::fmt::Debug for SceneItems {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SceneItems")
    }
}

/// One open chooser.
#[derive(Clone, Debug)]
pub struct Chooser {
    pub purpose: Purpose,
    pub title: String,
    pub list: Option<ListKey>,
    /// Items given by the caller instead of a list (a detail object's colour picks from the
    /// dialog's own objects).
    pub items: Option<Arc<Vec<String>>>,
    /// A list the open scene gives (`gz-scene`'s `scene_items`): its way paths, its story ids.
    pub scene_list: Option<SmolStr>,
    pub multi: bool,
    pub search: String,
    pub picked: Vec<String>,
    /// The value it started from.
    pub current: String,
    /// The item the preview shows and the keys move from.
    pub highlight: Option<String>,
    /// Scroll the highlighted row into view on the next frame.
    reveal: bool,
    order: Option<Order>,
    scene_items: Option<SceneItems>,
    /// What the last frame showed: the count and the first rows, for `level.show`.
    shown: usize,
    first: Vec<String>,
    /// The search took the keys when the chooser came up.
    focused: bool,
}

impl Chooser {
    #[must_use]
    pub fn new(purpose: Purpose, what: &str, list: Option<ListKey>, current: &str) -> Self {
        Self {
            purpose,
            title: format!("Select {what}"),
            list,
            items: None,
            scene_list: None,
            multi: false,
            search: String::new(),
            picked: if current.is_empty() { Vec::new() } else { vec![current.to_owned()] },
            current: current.to_owned(),
            highlight: (!current.is_empty()).then(|| current.to_owned()),
            reveal: true,
            order: None,
            scene_items: None,
            shown: 0,
            first: Vec::new(),
            focused: false,
        }
    }

    #[must_use]
    pub fn multi(mut self) -> Self {
        self.multi = true;
        self.picked.clear();
        self
    }

    /// Takes several items, starting from those of the comma list it was opened on (a model's
    /// fixed bones).
    #[must_use]
    pub fn multi_from_current(mut self) -> Self {
        self.multi = true;
        self.picked =
            self.current.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
        self
    }

    #[must_use]
    pub fn with_items(mut self, items: Vec<String>) -> Self {
        self.items = Some(Arc::new(items));
        self
    }

    /// Items the open scene gives, by the list's name (`props::scene_items`).
    #[must_use]
    pub fn with_scene_list(mut self, list: &str) -> Self {
        self.scene_list = Some(SmolStr::new(list));
        self
    }

    /// Moves the highlight (and, choosing one item, the pick) to `item`.
    pub fn highlight(&mut self, item: &str) {
        self.highlight = Some(item.to_owned());
        if !self.multi {
            self.picked = vec![item.to_owned()];
        }
        self.reveal = true;
    }

    /// What its pictures are of: models, library objects, textures; `None` for a list of names.
    #[must_use]
    pub fn pictures(&self) -> Option<PicKind> {
        match self.list {
            Some(ListKey::Visuals) => Some(PicKind::Visual),
            Some(ListKey::Objects) => Some(PicKind::Object),
            Some(ListKey::Textures) => Some(PicKind::Texture),
            _ => None,
        }
    }

    /// Whether the highlighted item can be played.
    #[must_use]
    pub fn sounds(&self) -> bool {
        self.list == Some(ListKey::Sounds)
    }

    /// What `level.show` and `scene.choose` say of it.
    #[must_use]
    pub fn state(&self) -> Value {
        json!({
            "title": self.title,
            "list": self.list.as_ref().map(ListKey::noun),
            "query": self.search,
            "shown": self.shown,
            "first": self.first,
            "highlight": self.highlight,
            "picked": self.picked,
            "current": self.current,
            "several": self.multi,
            "preview": match (self.pictures(), self.sounds()) {
                (Some(PicKind::Visual | PicKind::Object), _) => "model",
                (Some(PicKind::Texture), _) => "texture",
                (None, true) => "sound",
                (None, false) => "none",
            },
        })
    }
}

/// What the dialog did this frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Open,
    Cancelled,
    /// Picked: the items (one unless the chooser takes several; empty for None).
    Picked(Vec<String>),
}

/// The last part of a list path: `props\box_01` is `box_01`.
#[must_use]
pub fn leaf(item: &str) -> &str {
    item.rsplit(['\\', '/']).next().unwrap_or(item)
}

/// The folder of a list path, empty at the top.
#[must_use]
pub fn folder(item: &str) -> &str {
    item.rfind(['\\', '/']).map_or("", |at| &item[..at])
}

/// The items' order for a search: as listed without one, ranked by the leaf name, then the path.
#[must_use]
pub fn order_for(items: &[String], search: &str) -> Vec<u32> {
    gz_ui::rank::rank(items.len(), search, |i| {
        let it = items.get(i).map_or("", String::as_str);
        (leaf(it), it, "")
    })
    .into_iter()
    .filter_map(|i| u32::try_from(i).ok())
    .collect()
}

fn text_rev(s: &str) -> u64 {
    use std::hash::{BuildHasher, BuildHasherDefault};
    BuildHasherDefault::<rustc_hash::FxHasher>::default().hash_one(s)
}

/// The items to choose from, when they are there.
fn items_of(editor: &Arc<Editor>, c: &mut Chooser) -> Option<Arc<Vec<String>>> {
    if let Some(items) = &c.items {
        return Some(items.clone());
    }
    if let Some(list) = c.scene_list.clone() {
        let svc = crate::actions::scene(editor)?;
        let snap = svc.store.snapshot();
        let q = c.scene_items.get_or_insert_with(|| SceneItems(Query::new(editor.exec.clone())));
        let answer =
            q.0.get(&(), snap.rev, move || gz_scene::props::scene_items(&snap, &list).unwrap_or_default());
        return answer.value().map(|v| Arc::new(v.as_ref().clone()));
    }
    match &c.list {
        Some(key) => Lists::get(editor, key),
        // a list its owner did not give: nothing to pick
        None => Some(Arc::new(Vec::new())),
    }
}

/// The rows' height: a picture's row holds a 40 pt picture.
fn row_height(pictures: bool) -> f32 {
    if pictures { 46.0 } else { gz_ui::metrics::ROW + 4.0 }
}

const PREVIEW_W: f32 = 240.0;

/// Draws the chooser. The caller keeps it while [`Outcome::Open`] comes back. `gpu` makes the
/// pictures of models.
pub fn draw(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    gpu: Option<(&wgpu::Device, &wgpu::Queue)>,
    c: &mut Chooser,
) -> Outcome {
    let ctx = ui.ctx().clone();
    let frame = ctx.cumulative_frame_nr();
    let pics = Pics::of(editor);
    pics.begin_frame(&ctx, frame);
    let mut open = true;
    let mut outcome = Outcome::Open;
    let items = items_of(editor, c);
    let order = items.as_ref().map(|items| {
        let q = c.order.get_or_insert_with(|| Order(Query::new(editor.exec.clone()))).0.clone();
        let rev = Rev(Arc::as_ptr(items) as usize as u64).mix(Rev(text_rev(&c.search)));
        let (list, search) = (items.clone(), c.search.clone());
        q.get(&(), rev, move || order_for(&list, &search))
    });
    let lines: Option<Arc<Vec<u32>>> = order.as_ref().and_then(|a| a.value().cloned());
    let mut double = None;
    let any = !c.picked.is_empty();
    let status = if c.current.is_empty() { String::new() } else { format!("Now: {}", leaf(&c.current)) };
    let multi = c.multi;
    let pictures = c.pictures();
    let previews = pictures.is_some() || c.sounds();
    let (mut select, mut none, mut cancel) = (false, false, false);
    // most of the window's height: thousands of rows want room
    let body_h = (ctx.content_rect().height() * 0.72 - 120.0).clamp(280.0, 760.0);

    // the keys move the highlight before the search field sees them
    let (up, down, page_up, page_down) = ctx.input_mut(|i| {
        (
            i.consume_key(Modifiers::NONE, Key::ArrowUp),
            i.consume_key(Modifiers::NONE, Key::ArrowDown),
            i.consume_key(Modifiers::NONE, Key::PageUp),
            i.consume_key(Modifiers::NONE, Key::PageDown),
        )
    });

    let title = c.title.clone();
    Dialog::new(&title, "dlg.choose", DialogSize::L).show(
        ui,
        &mut open,
        |ui| {
            let width = ui.available_width();
            let list_w = if previews { width - PREVIEW_W - 12.0 } else { width };
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    Vec2::new(list_w, body_h),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(list_w);
                        let resp = SearchField::new(&mut c.search)
                            .hint(&format!("Search {}", c.list.as_ref().map_or("items", ListKey::noun)))
                            .width(list_w)
                            .id_salt("choose_search")
                            .show(ui);
                        if !std::mem::replace(&mut c.focused, true) {
                            resp.request_focus();
                        }
                        ui.add_space(4.0);
                        let list_h =
                            (body_h - gz_ui::metrics::FRAME - 8.0 - if multi { 24.0 } else { 0.0 }).max(80.0);
                        match (&items, &lines) {
                            (None, _) | (Some(_), None) => state::busy_line(
                                ui,
                                &format!("Reading the {}...", c.list.as_ref().map_or("items", ListKey::noun)),
                            ),
                            (Some(list), Some(_)) if list.is_empty() => {
                                state::empty_state(
                                    ui,
                                    Icon::Search,
                                    "Nothing to pick",
                                    &c.list.as_ref().map_or_else(
                                        || "No items were found.".to_owned(),
                                        ListKey::empty_text,
                                    ),
                                    None,
                                );
                            }
                            (Some(list), Some(lines)) => {
                                let moved = step_highlight(c, list, lines, up, down, page_up, page_down);
                                c.reveal |= moved;
                                if let Some(item) =
                                    rows(ui, editor, gpu, &pics, c, list, lines, list_w, list_h, frame)
                                {
                                    let again = ui.input(|i| {
                                        i.pointer.button_double_clicked(egui::PointerButton::Primary)
                                    });
                                    if c.multi {
                                        if let Some(i) = c.picked.iter().position(|p| *p == item) {
                                            c.picked.remove(i);
                                        } else {
                                            c.picked.push(item.clone());
                                        }
                                        c.highlight = Some(item.clone());
                                    } else {
                                        c.highlight(&item);
                                        c.reveal = false;
                                    }
                                    if again && !c.multi {
                                        double = Some(item);
                                    }
                                }
                                c.shown = lines.len();
                                c.first = lines
                                    .iter()
                                    .take(20)
                                    .filter_map(|i| list.get(*i as usize).cloned())
                                    .collect();
                                if c.multi {
                                    ui.add_space(4.0);
                                    let picked = if c.picked.is_empty() {
                                        "Nothing picked yet.".to_owned()
                                    } else {
                                        format!("{} picked: {}", c.picked.len(), c.picked.join(", "))
                                    };
                                    text::label(
                                        ui,
                                        &picked,
                                        Font::Small,
                                        palette(ui.ctx()).text_secondary,
                                        Clip::End,
                                    );
                                }
                            }
                        }
                    },
                );
                if previews {
                    ui.add_space(12.0);
                    ui.allocate_ui_with_layout(
                        Vec2::new(PREVIEW_W, body_h),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(PREVIEW_W);
                            preview(ui, editor, gpu, &pics, c, frame);
                        },
                    );
                }
            });
        },
        |f| {
            f.status(&status, Tone::Muted);
            select = f.button("Select", Kind::Primary, true, any, "Pick an item first.");
            none = !multi && f.button("None", Kind::Secondary, false, true, "");
            cancel = f.button("Cancel", Kind::Secondary, false, true, "");
        },
    );
    if select {
        outcome = Outcome::Picked(c.picked.clone());
    } else if none {
        outcome = Outcome::Picked(Vec::new());
    } else if cancel {
        outcome = Outcome::Cancelled;
    }
    if let Some(item) = double {
        return Outcome::Picked(vec![item]);
    }
    if !open && outcome == Outcome::Open {
        if c.sounds() {
            gz_ui_previews::audio::Player::of(editor).stop();
        }
        return Outcome::Cancelled;
    }
    if outcome != Outcome::Open && c.sounds() {
        gz_ui_previews::audio::Player::of(editor).stop();
    }
    outcome
}

/// Moves the highlight by the keys pressed; whether it moved.
fn step_highlight(
    c: &mut Chooser,
    list: &[String],
    lines: &[u32],
    up: bool,
    down: bool,
    page_up: bool,
    page_down: bool,
) -> bool {
    let n = lines.len();
    if n == 0 || !(up || down || page_up || page_down) {
        return false;
    }
    let at = c.highlight.as_ref().and_then(|h| lines.iter().position(|i| list.get(*i as usize) == Some(h)));
    let step: isize = if down {
        1
    } else if up {
        -1
    } else if page_down {
        10
    } else {
        -10
    };
    let next = match at {
        None => 0,
        Some(at) => (at as isize + step).clamp(0, n as isize - 1) as usize,
    };
    if let Some(item) = lines.get(next).and_then(|i| list.get(*i as usize)).cloned() {
        if c.multi {
            c.highlight = Some(item);
        } else {
            c.highlight(&item);
        }
        return true;
    }
    false
}

/// The rows in view; the item clicked.
#[allow(clippy::too_many_arguments)]
fn rows(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    gpu: Option<(&wgpu::Device, &wgpu::Queue)>,
    pics: &Arc<Pics>,
    c: &mut Chooser,
    list: &[String],
    lines: &[u32],
    width: f32,
    height: f32,
    frame: u64,
) -> Option<String> {
    let p = palette(ui.ctx());
    let pictures = c.pictures();
    let row_h = row_height(pictures.is_some());
    let ppp = ui.ctx().pixels_per_point();
    let mut clicked = None;
    let mut scroll =
        egui::ScrollArea::vertical().id_salt("choose_rows").max_height(height).auto_shrink([false, false]);
    if std::mem::take(&mut c.reveal)
        && let Some(at) =
            c.highlight.as_ref().and_then(|h| lines.iter().position(|i| list.get(*i as usize) == Some(h)))
    {
        // the row in the middle of the view
        scroll = scroll.vertical_scroll_offset((at as f32 * row_h - height * 0.5 + row_h * 0.5).max(0.0));
    }
    let frame_rect = Rect::from_min_size(ui.cursor().min, Vec2::new(width, height));
    ui.painter().rect_filled(frame_rect, egui::CornerRadius::same(4), p.inset);
    scroll.show_rows(ui, row_h, lines.len(), |ui, range| {
        for n in range {
            let Some(item) = lines.get(n).and_then(|i| list.get(*i as usize)) else { continue };
            let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::click());
            let hi = c.highlight.as_deref() == Some(item.as_str());
            let picked = c.picked.iter().any(|x| x == item);
            if hi {
                ui.painter().rect_filled(rect, egui::CornerRadius::same(4), p.selection_focused);
            } else if picked {
                ui.painter().rect_filled(rect, egui::CornerRadius::same(4), p.selection);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect, egui::CornerRadius::same(4), p.hover);
            }
            let mut x = rect.min.x + 8.0;
            if let Some(kind) = pictures {
                let pic = Rect::from_min_size(pos2(x, rect.min.y + 3.0), Vec2::splat(row_h - 6.0));
                match pics.get(editor, gpu, kind, item, ((row_h - 6.0) * ppp) as u32, frame) {
                    Some(tex) => {
                        ui.painter().image(
                            tex,
                            pic,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                            egui::Color32::WHITE,
                        );
                    }
                    None => {
                        ui.painter().rect_filled(pic, egui::CornerRadius::same(4), p.raised);
                        let glyph =
                            if kind == PicKind::Texture { Icon::FileTexture } else { Icon::FileModel };
                        gz_ui::icons::draw_in(ui.painter(), glyph, pic, p.text_muted);
                    }
                }
                x = pic.max.x + 10.0;
                let room = rect.max.x - x - 8.0;
                text::paint(
                    ui.painter(),
                    &Font::Body.id(),
                    pos2(x, rect.center().y - 9.0),
                    egui::Align2::LEFT_CENTER,
                    p.text,
                    leaf(item),
                    Some(room),
                    Clip::End,
                );
                text::paint(
                    ui.painter(),
                    &Font::Small.id(),
                    pos2(x, rect.center().y + 10.0),
                    egui::Align2::LEFT_CENTER,
                    p.text_muted,
                    folder(item),
                    Some(room),
                    Clip::Start,
                );
            } else {
                let name_w = text::width(ui.ctx(), &Font::Body.id(), leaf(item)).min(width * 0.6);
                text::paint(
                    ui.painter(),
                    &Font::Body.id(),
                    pos2(x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    p.text,
                    leaf(item),
                    Some(width * 0.6),
                    Clip::End,
                );
                x += name_w + 12.0;
                text::paint(
                    ui.painter(),
                    &Font::Small.id(),
                    pos2(x, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    p.text_muted,
                    folder(item),
                    Some((rect.max.x - x - 28.0).max(0.0)),
                    Clip::Start,
                );
            }
            if *item == c.current {
                let tick =
                    Rect::from_center_size(pos2(rect.max.x - 16.0, rect.center().y), Vec2::splat(14.0));
                gz_ui::icons::draw(ui.painter(), Icon::Check, tick, p.accent_bright);
            }
            gz_ui::audit::control(ui.ctx(), leaf(item), true, None, None, rect);
            let resp = resp.on_hover_text(item.as_str());
            if resp.clicked() {
                clicked = Some(item.clone());
            }
        }
    });
    clicked
}

/// The preview of the highlighted item: its picture, its name and folder, Play for a sound.
fn preview(
    ui: &mut Ui,
    editor: &Arc<Editor>,
    gpu: Option<(&wgpu::Device, &wgpu::Queue)>,
    pics: &Arc<Pics>,
    c: &Chooser,
    frame: u64,
) {
    let p = palette(ui.ctx());
    let Some(item) = c.highlight.clone() else {
        state::empty_state(ui, Icon::Picture, "Nothing highlighted", "Pick a row to see it here.", None);
        return;
    };
    let side = PREVIEW_W;
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    ui.painter().rect_filled(rect, egui::CornerRadius::same(6), p.raised);
    match c.pictures() {
        Some(kind) => {
            let px = (side * ui.ctx().pixels_per_point()) as u32;
            match pics.get(editor, gpu, kind, &item, px, frame) {
                Some(tex) => {
                    ui.painter().image(
                        tex,
                        rect.shrink(4.0),
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        egui::Color32::WHITE,
                    );
                }
                None => {
                    let at = Rect::from_center_size(rect.center(), Vec2::splat(24.0));
                    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(at));
                    state::spinner(&mut child);
                }
            }
        }
        None => {
            gz_ui::icons::draw_in(ui.painter(), Icon::FileSound, rect.shrink(side * 0.3), p.text_secondary)
        }
    }
    ui.add_space(8.0);
    text::label(ui, leaf(&item), Font::Strong, p.text, Clip::End).on_hover_text(&item);
    text::label(ui, folder(&item), Font::Small, p.text_muted, Clip::Start);
    if c.sounds() {
        ui.add_space(8.0);
        let player = gz_ui_previews::audio::Player::of(editor);
        let playing = player.is_playing();
        ui.horizontal(|ui| {
            if gz_ui::Button::new("Play").icon(Icon::Play).show(ui).clicked() {
                play(editor, &item);
            }
            if gz_ui::Button::new("Stop")
                .icon(Icon::Stop)
                .enabled(playing, "Nothing plays.")
                .show(ui)
                .clicked()
            {
                player.stop();
            }
        });
        if let Some(e) = player.error() {
            text::label(ui, &e, Font::Small, p.tone(Tone::Error), Clip::End);
        }
    }
}

/// Plays a game sound, decoded off the UI thread.
fn play(editor: &Arc<Editor>, item: &str) {
    let ed = editor.clone();
    let rel = format!("sounds/{}.ogg", item.replace('\\', "/").to_lowercase());
    editor.exec.spawn(async move {
        let Ok(link) = gz_game::GameLink::of(&ed) else { return };
        let Ok(files) = link.files().await else { return };
        let pcm =
            ed.exec.io(move || files.read(&rel).ok().and_then(|b| gz_fmt_sound::decode_ogg(&b).ok())).await;
        match pcm {
            Some(pcm) => gz_ui_previews::audio::Player::of(&ed).play(Arc::new(pcm)),
            None => ed.notices.error("The sound could not be read.".to_owned()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn a_search_ranks_the_leaf_names_first() {
        let list =
            items(&["props\\barrel_metal", "furniture\\bar_stool", "props\\crate_bar", "lights\\lamp"]);
        let all = order_for(&list, "");
        assert_eq!(all, vec![0, 1, 2, 3]);
        let bar = order_for(&list, "bar");
        // whole-word prefixes of the leaf first; the lamp holds no "bar"
        assert_eq!(bar.first().copied(), Some(1));
        assert!(!bar.contains(&3));
        assert_eq!(leaf("props\\barrel_metal"), "barrel_metal");
        assert_eq!(folder("props\\barrel_metal"), "props");
        assert_eq!(folder("lamp"), "");
    }

    #[test]
    fn the_keys_move_the_highlight_and_the_pick() {
        let list = items(&["a\\one", "a\\two", "b\\three"]);
        let lines = vec![0, 1, 2];
        let mut c = Chooser::new(Purpose::DetailObjects, "Object", Some(ListKey::Objects), "a\\one");
        assert!(step_highlight(&mut c, &list, &lines, false, true, false, false));
        assert_eq!(c.highlight.as_deref(), Some("a\\two"));
        assert_eq!(c.picked, vec!["a\\two".to_owned()]);
        assert!(step_highlight(&mut c, &list, &lines, false, false, false, true));
        assert_eq!(c.highlight.as_deref(), Some("b\\three"));
        assert_eq!(c.pictures(), Some(PicKind::Object));
        let s = c.state();
        assert_eq!(s["preview"], "model");
        assert_eq!(s["current"], "a\\one");
    }

    /// The chooser draws its list and its preview in a frame of the kit: the rows of a list of
    /// models carry pictures, the preview names the highlighted item.
    #[test]
    fn the_list_and_its_preview_draw() {
        let rt =
            gz_task::Runtime::start(gz_task::ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 1 })
                .expect("runtime");
        let ed = gz_editor::Editor::new(
            gz_editor::EditorOptions::default(),
            rt.exec(),
            gz_log::Log::memory(),
            gz_editor::Registry::new(),
        );
        let mut c = Chooser::new(Purpose::DetailObjects, "Model", Some(ListKey::Visuals), "")
            .with_items(items(&["dynamics\\barrel", "dynamics\\box", "statics\\wall"]));
        c.highlight("dynamics\\box");
        let ctx = egui::Context::default();
        gz_ui::theme::install(&ctx, gz_ui::theme::Accent::default());
        let mut outcome = Outcome::Open;
        // the order is made on the CPU pool: a few frames until it is there
        for t in 0..40 {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), Vec2::new(1600.0, 900.0))),
                time: Some(f64::from(t) * 0.05),
                ..egui::RawInput::default()
            };
            let mut out = ctx.run_ui(input, |ui| outcome = draw(ui, &ed, None, &mut c));
            out.textures_delta.clear();
            if c.shown == 3 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(outcome, Outcome::Open);
        assert_eq!(c.shown, 3);
        assert_eq!(c.first, items(&["dynamics\\barrel", "dynamics\\box", "statics\\wall"]));
        assert_eq!(c.state()["preview"], "model");
        assert_eq!(c.picked, vec!["dynamics\\box".to_owned()]);
        drop(rt);
    }
}
