//! Dialogs (docs/UI_DESIGN.md, section 4, "Dialogs" and "Precedence").
//!
//! - [`Dialog::new(title, "dlg.<name>", size)`](Dialog::new)`.show(ui, &mut open, |body|, |footer|)`
//!   - a modal over the dim colour, centred (clamped once the author moved it), at most W-32 x
//!     H-48: a 36 pt title with a close cross that takes no keyboard focus, a body that scrolls, a
//!     52 pt footer that stays put. It fits what it holds while it opens and keeps that height:
//!     a running job never resizes it. [`Dialog::sheet`] raises it from the dialog being drawn:
//!     under its parent's title, no wider than it, the parent dimmed.
//! - [`Footer`] - status on the left ([`Footer::status`], or [`Footer::busy`] while a job runs),
//!   buttons on the right ([`Footer::button`], at least 88 x 28). Enter presses the default, Esc
//!   cancels; in a destructive dialog (one with a Danger button) both cancel.
//! - Queued kit dialogs, drawn at the end of the frame by [`draw_overlays`]: [`confirm`],
//!   [`unsaved_prompt`], [`notice`], [`result`]. Who answers is the caller's [`Asker`]: `Mcp`
//!   answers a confirm Yes at once and opens nothing, `Unattended` answers No (a notice OK) and
//!   the function returns the line to log, `Ui` opens the dialog. `then` runs after the frame
//!   that answered, outside every window.
//! - For MCP: [`open_dialogs`] (what the last frame drew, topmost last), [`modal_on_top`],
//!   [`close_dialog`] (as Esc).

use std::sync::Arc;

use egui::{
    Align, Align2, Area, Context, CornerRadius, Frame, Id, Key, Layout, Margin, Modal, Modifiers, Order,
    Popup, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, UiBuilder, UiKind, Vec2, pos2,
};
use gz_base::{Finding, Severity};
use parking_lot::Mutex;

use crate::audit;
use crate::button::{Button, Kind};
use crate::font::Font;
use crate::icons::{self, Icon};
use crate::state::{paint_spinner, tone_icon};
use crate::text::{self, Clip};
use crate::theme::{Tone, metrics, palette};
use crate::units::{button_width, em};

/// A dialog's width class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DialogSize {
    /// 400 pt.
    S,
    /// 560 pt.
    #[default]
    M,
    /// 760 pt.
    L,
    /// 1040 pt or 90% of the window, whichever is less.
    XL,
}

impl DialogSize {
    /// The width in points for a work area `work_w` wide.
    #[must_use]
    pub fn width(self, work_w: f32) -> f32 {
        match self {
            DialogSize::S => metrics::DIALOG_S,
            DialogSize::M => metrics::DIALOG_M,
            DialogSize::L => metrics::DIALOG_L,
            DialogSize::XL => metrics::DIALOG_XL.min(work_w * 0.9),
        }
    }
}

/// How a question was answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Cancel,
}

/// Who answers a question.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asker {
    /// A person: the dialog opens.
    Ui,
    /// An MCP caller, who has decided: a confirmation is Yes, nothing opens.
    Mcp,
    /// `-nodlg`: the logged answers - No for a question, OK for a note.
    Unattended,
}

/// A dialog the last frame drew.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct DialogInfo {
    /// `dlg.<name>`.
    pub id: String,
    pub title: String,
    /// Its rectangle in pixels: min x, min y, max x, max y.
    pub rect_px: [f32; 4],
}

/// A confirmation (`confirm`).
#[derive(Clone, Debug, Default)]
pub struct ConfirmSpec {
    /// The name after `dlg.`.
    pub id: String,
    pub title: String,
    /// The question, a sentence naming what happens.
    pub headline: String,
    /// What it touches: up to 10 are listed, then "and N more".
    pub items: Vec<String>,
    /// How many there are in all, when `items` lists only some.
    pub total: usize,
    /// What it costs, in the warning colour; a destructive one says "Deleted for good" without it.
    pub consequence: String,
    /// The action button's label; "Delete" for a destructive one, "OK" otherwise.
    pub action: String,
    /// Loses data: the action is Danger, Cancel the default, Enter and Esc cancel.
    pub destructive: bool,
}

/// A button of a [`ResultSpec`] that does something without closing it (Open Folder).
#[derive(Clone)]
pub struct ResultAction {
    pub label: String,
    pub run: Arc<dyn Fn() + Send + Sync>,
}

impl std::fmt::Debug for ResultAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResultAction").field("label", &self.label).finish_non_exhaustive()
    }
}

/// The result of a build, an export, a package, a publish or a bake (`result`).
#[derive(Clone, Debug)]
pub struct ResultSpec {
    /// The name after `dlg.`: `build_result`, `export_result`, ...
    pub id: String,
    pub title: String,
    pub tone: Tone,
    pub headline: String,
    /// Key and value pairs.
    pub rows: Vec<(String, String)>,
    /// What the build passed by: "Not shipped".
    pub left_out: Vec<String>,
    /// Up to 8 are shown.
    pub warnings: Vec<String>,
    pub action: Option<ResultAction>,
}

impl ResultSpec {
    /// A result with no rows, nothing left out, no warnings and no action.
    #[must_use]
    pub fn new(id: &str, title: &str, tone: Tone, headline: &str) -> Self {
        Self {
            id: id.to_owned(),
            title: title.to_owned(),
            tone,
            headline: headline.to_owned(),
            rows: Vec::new(),
            left_out: Vec::new(),
            warnings: Vec::new(),
            action: None,
        }
    }
}

type Then = Box<dyn FnOnce(Answer) + Send>;

enum QKind {
    Confirm(ConfirmSpec),
    Unsaved { headline: String, items: Vec<String> },
    Notice { tone: Tone, text: String },
    Result(ResultSpec),
}

struct Queued {
    kind: QKind,
    title: String,
    name: String,
    /// The dialog this one is a sheet of, while that dialog is drawn.
    parent: Option<String>,
    open: bool,
    answer: Option<Answer>,
    then: Option<Then>,
}

/// A dialog being drawn: sheets raised now belong to it.
#[derive(Clone, Debug)]
struct Drawn {
    name: String,
    rect: Rect,
}

#[derive(Default)]
struct Shared {
    queue: Vec<Queued>,
    stack: Vec<Drawn>,
    cur: Vec<DialogInfo>,
    last: Vec<DialogInfo>,
    close_requests: Vec<String>,
    actions: Vec<Arc<dyn Fn() + Send + Sync>>,
    /// Who answers every question while a caller's scope is held.
    caller: Option<Asker>,
}

#[derive(Clone, Default)]
struct Handle(Arc<Mutex<Shared>>);

fn handle(ctx: &Context) -> Handle {
    let id = Id::unique("gz-ui.dialogs");
    ctx.data(|d| d.get_temp::<Handle>(id)).unwrap_or_else(|| {
        let made = Handle::default();
        ctx.data_mut(|d| d.insert_temp(id, made.clone()));
        made
    })
}

fn work_area_id() -> Id {
    Id::unique("gz-ui.dialog_work_area")
}

/// The window's work area (the shell's, below its toolbar and above its status bar), set every
/// frame: a dialog is centred and held inside it.
pub fn set_work_area(ctx: &Context, rect: Rect) {
    ctx.data_mut(|d| d.insert_temp(work_area_id(), rect));
}

/// The work area a shell set, else the whole window.
fn work_area(ctx: &Context) -> Rect {
    let whole = ctx.content_rect();
    match ctx.data(|d| d.get_temp::<Rect>(work_area_id())) {
        Some(r) if r.is_positive() && r.height() > 120.0 => r.intersect(whole),
        _ => whole,
    }
}

/// `dlg.<name>` for a name given with or without the prefix.
fn full_name(name: &str) -> String {
    if name.starts_with("dlg.") { name.to_owned() } else { format!("dlg.{name}") }
}

/// What a dialog keeps between frames.
#[derive(Clone, Debug)]
struct Mem {
    shown: u32,
    last_frame: u64,
    desired_h: f32,
    footer_w: f32,
    danger: bool,
    moved: bool,
    pos: Option<Pos2>,
    work: Rect,
    buttons: Vec<Id>,
    default_button: Option<Id>,
    /// The body's height the window was last sized by.
    measured: f32,
    /// When the author last pressed, released or typed: a change of height that follows is
    /// theirs, and the window keeps its size under them.
    touched: f64,
}

impl Default for Mem {
    fn default() -> Self {
        Self {
            shown: 0,
            last_frame: 0,
            desired_h: 0.0,
            footer_w: 0.0,
            danger: false,
            moved: false,
            pos: None,
            work: Rect::NOTHING,
            buttons: Vec::new(),
            default_button: None,
            measured: 0.0,
            touched: f64::NEG_INFINITY,
        }
    }
}

const TITLE_H: f32 = metrics::DIALOG_TITLE;
const FOOTER_H: f32 = metrics::DIALOG_FOOTER;
/// The frames a dialog measures its height on.
const MEASURE_FRAMES: u32 = 2;
/// How long after the author's last press or key a change of height stays theirs.
const QUIET_S: f64 = 0.6;

/// The footer of a dialog: the status on the left, the buttons on the right.
pub struct Footer<'u> {
    ui: &'u mut Ui,
    rect: Rect,
    last_w: f32,
    x: f32,
    y: f32,
    w_cur: f32,
    danger: bool,
    /// Enter may press the default this frame.
    enter_ready: bool,
    /// The dialog has just opened: the default takes the keyboard focus.
    opening: bool,
    buttons: Vec<Id>,
    default_button: Option<Id>,
    /// A job runs: the window keeps its height.
    busy: bool,
}

impl Footer<'_> {
    /// A status line on the left with its tone's icon, cut to what the buttons leave.
    pub fn status(&mut self, text_: &str, tone: Tone) {
        if text_.is_empty() {
            return;
        }
        let ctx = self.ui.ctx().clone();
        let p = palette(&ctx);
        let x = self.rect.min.x + 16.0;
        let cy = self.rect.center().y;
        let icon = metrics::GLYPH_ROW;
        icons::draw(
            self.ui.painter(),
            tone_icon(tone),
            Rect::from_min_size(pos2(x, cy - icon * 0.5), Vec2::splat(icon)),
            p.tone(tone),
        );
        let room = (self.rect.width() - 32.0 - self.last_w - 16.0 - icon - 6.0).max(0.0);
        let color = if tone == Tone::Muted { p.text_secondary } else { p.tone(tone) };
        let tx = x + icon + 6.0;
        let cut = text::paint(
            self.ui.painter(),
            &Font::Body.id(),
            pos2(tx, cy),
            Align2::LEFT_CENTER,
            color,
            text_,
            Some(room),
            Clip::End,
        );
        if cut {
            let r = Rect::from_min_size(pos2(tx, cy - 8.0), Vec2::new(room, 16.0));
            self.ui.interact(r, self.ui.scope_id().with("status"), Sense::hover()).on_hover_text(text_);
        }
    }

    /// While a job runs: a spinner and its stage on the left. The job's stop is a labelled
    /// button of the footer (`Stop <job>`). An audited frame reports it, like a busy line.
    pub fn busy(&mut self, stage: &str) {
        self.busy = true;
        let ctx = self.ui.ctx().clone();
        let p = palette(&ctx);
        let x = self.rect.min.x + 16.0;
        let cy = self.rect.center().y;
        paint_spinner(
            &ctx,
            self.ui.painter(),
            Rect::from_center_size(pos2(x + 8.0, cy), Vec2::splat(metrics::GLYPH)),
        );
        let tx = x + metrics::GLYPH + 8.0;
        let buttons_x = self.rect.max.x - 16.0 - self.last_w;
        let room = (buttons_x - 16.0 - tx).max(em(3.0));
        let stage = if stage.is_empty() { "Working..." } else { stage };
        text::paint(
            self.ui.painter(),
            &Font::Body.id(),
            pos2(tx, cy),
            Align2::LEFT_CENTER,
            p.text,
            stage,
            Some(room),
            Clip::End,
        );
        audit::loading(
            &ctx,
            stage,
            Rect::from_min_max(pos2(x, self.rect.min.y), pos2(buttons_x.max(tx), self.rect.max.y)),
        );
    }

    /// A button, placed after the ones before it. `is_default` takes the keyboard focus when the
    /// dialog opens and is pressed by Enter (never a Danger one, never in a destructive dialog).
    pub fn button(&mut self, label: &str, kind: Kind, is_default: bool, enabled: bool, why: &str) -> bool {
        self.button_set(label, &[], kind, is_default, enabled, why)
    }

    /// A button whose label changes with the state: it keeps the width of the widest of
    /// `label_set`.
    pub fn button_set(
        &mut self,
        label: &str,
        label_set: &[&str],
        kind: Kind,
        is_default: bool,
        enabled: bool,
        why: &str,
    ) -> bool {
        let ctx = self.ui.ctx().clone();
        let w = label_set
            .iter()
            .chain(std::iter::once(&label))
            .map(|l| button_width(&ctx, l, kind, false, metrics::DIALOG_BUTTON_W))
            .fold(0.0, f32::max);
        if self.w_cur > 0.0 {
            self.x += 8.0;
            self.w_cur += 8.0;
        }
        let rect = Rect::from_min_size(pos2(self.x, self.y), Vec2::new(w, metrics::DIALOG_BUTTON_H));
        self.x += w;
        self.w_cur += w;
        if kind == Kind::Danger {
            self.danger = true;
        }
        if kind == Kind::Primary {
            crate::button::note_primary(&ctx, self.ui.scope_id(), "dialog footer", label);
        }
        let resp = self
            .ui
            .put(rect, Button::new(label).kind(kind).enabled(enabled, why).label_set(label_set).dialog());
        self.buttons.push(resp.id);
        let mut pressed = resp.clicked() && enabled;
        if is_default && enabled && kind != Kind::Danger {
            self.default_button = Some(resp.id);
            // the keyboard's focus opens on the default, the safe answer
            if self.opening && ctx.memory(|m| m.focused().is_none()) {
                resp.request_focus();
            }
            if !pressed && self.enter_ready && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                pressed = true;
            }
        }
        pressed
    }
}

/// A modal dialog.
#[must_use = "a dialog does nothing until it is shown: .show(ui, &mut open, ..)"]
pub struct Dialog<'a> {
    title: &'a str,
    name: &'a str,
    size: DialogSize,
    sheet: bool,
}

impl<'a> Dialog<'a> {
    /// `name` is `dlg.<name>` (the prefix is added when missing): what MCP calls it.
    pub fn new(title: &'a str, name: &'a str, size: DialogSize) -> Self {
        Self { title, name, size, sheet: false }
    }

    /// Raised from the dialog being drawn: under its title, no wider than it. Outside a dialog it
    /// is a dialog of its own.
    pub fn sheet(mut self) -> Self {
        self.sheet = true;
        self
    }

    /// Shows the dialog while `open` is true; the close cross and Esc clear it. Returns what
    /// `body` returned, `None` while it is closed.
    pub fn show<R>(
        self,
        ui: &mut Ui,
        open: &mut bool,
        body: impl FnOnce(&mut Ui) -> R,
        footer: impl FnOnce(&mut Footer<'_>),
    ) -> Option<R> {
        let ctx = ui.ctx().clone();
        let parent = if self.sheet { handle(&ctx).0.lock().stack.last().cloned() } else { None };
        show_impl(&ctx, self.title, &full_name(self.name), self.size, parent, open, body, footer)
    }
}

#[allow(clippy::too_many_arguments)]
fn show_impl<R>(
    ctx: &Context,
    title: &str,
    name: &str,
    size: DialogSize,
    parent: Option<Drawn>,
    open: &mut bool,
    body: impl FnOnce(&mut Ui) -> R,
    footer: impl FnOnce(&mut Footer<'_>),
) -> Option<R> {
    let h = handle(ctx);
    let id = Id::unique(("gz-ui.dialog", name));
    {
        let mut s = h.0.lock();
        if let Some(i) = s.close_requests.iter().position(|n| n == name) {
            s.close_requests.remove(i);
            *open = false;
        }
    }
    if !*open {
        ctx.data_mut(|d| d.remove::<Mem>(id));
        return None;
    }
    let p = palette(ctx);
    let frame = ctx.cumulative_frame_nr();
    let mut mem = ctx.data(|d| d.get_temp::<Mem>(id)).unwrap_or_default();
    if mem.shown > 0 && mem.last_frame + 1 < frame {
        // not drawn last frame: this is an opening
        mem = Mem::default();
    }

    let work = work_area(ctx);
    let mut width = size.width(work.width()).min(work.width() - 32.0);
    let mut max_h = work.height() - 48.0;
    if let Some(par) = &parent {
        width = width.min(par.rect.width() - 32.0);
        max_h = max_h.min(par.rect.height() - TITLE_H - 16.0);
    }
    width = width.max(200.0);
    max_h = max_h.max(TITLE_H + FOOTER_H + 40.0);
    let height = if mem.desired_h > 0.0 { mem.desired_h.min(max_h) } else { max_h };
    let area_changed = mem.work != work;
    mem.work = work;
    let size_v = Vec2::new(width, height);
    let pos = match &parent {
        Some(par) => pos2(par.rect.center().x - width * 0.5, par.rect.min.y + TITLE_H),
        None => match mem.pos {
            // moved by the author: it stays where it was put, inside the window
            Some(at) if mem.moved => pos2(
                at.x.clamp(work.min.x, (work.max.x - width).max(work.min.x)),
                at.y.clamp(work.min.y, (work.max.y - height).max(work.min.y)),
            ),
            _ => work.center() - size_v * 0.5,
        },
    };
    if area_changed && !mem.moved {
        mem.pos = None;
    }
    mem.pos = Some(pos);

    // keys: never on the frame it opens, never under a sheet, never while a field or a popup has them
    let was_top = h.0.lock().last.last().is_some_and(|d| d.id == name);
    let focused = ctx.memory(|m| m.focused());
    let focus_free = focused.is_none_or(|f| mem.buttons.contains(&f));
    let keys_ok = mem.shown >= 1 && was_top && focus_free && !Popup::is_any_open(ctx);
    let mut close_now = false;
    if keys_ok && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        close_now = true;
    }
    // Enter cancels where the dialog would destroy something
    if keys_ok && mem.danger && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
        close_now = true;
    }
    let enter_ready = keys_ok && !mem.danger && focused.is_none_or(|f| Some(f) == mem.default_button);

    let area = Area::new(id)
        .kind(UiKind::Modal)
        .sense(Sense::hover())
        .order(Order::Foreground)
        .interactable(true)
        .fixed_pos(pos)
        .constrain(false);
    let mut content_h = 0.0;
    let mut out = None;
    let mut footer_state = (0.0, false, Vec::new(), None, false);
    let mut drawn_rect = Rect::from_min_size(pos, size_v);
    let opening = mem.shown <= 1;
    // where this dialog's record goes in the frame's list: before the sheets its body draws
    let mut cur_at = usize::MAX;
    let sheet = parent.is_some();
    let last_footer_w = mem.footer_w;
    let mut drag = Vec2::ZERO;
    Modal::new(id).area(area).backdrop_color(p.dim).frame(Frame::NONE).show(ctx, |ui| {
        let (rect, _) = ui.allocate_exact_size(size_v, Sense::hover());
        drawn_rect = rect;
        let radius = CornerRadius::same(if sheet { metrics::RADIUS_POPUP } else { metrics::RADIUS_DIALOG });
        ui.painter().rect(rect, radius, p.overlay, Stroke::new(1.0, p.border_strong), StrokeKind::Inside);

        // the title row: the name of the dialog, a close cross, and the grip to move it by
        let title_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), TITLE_H));
        let close_w = 40.0;
        if !sheet {
            let grip = Rect::from_min_max(title_rect.min, pos2(title_rect.max.x - close_w, title_rect.max.y));
            let g = ui.interact(grip, id.with("grip"), Sense::DRAG);
            if g.dragged() {
                drag = g.drag_delta();
            }
        }
        let room = title_rect.width() - 16.0 - close_w;
        text::paint(
            ui.painter(),
            &Font::Title.id(),
            pos2(title_rect.min.x + 16.0, title_rect.center().y),
            Align2::LEFT_CENTER,
            p.text,
            title,
            Some(room),
            Clip::End,
        );
        ui.painter().line_segment(
            [pos2(title_rect.min.x, title_rect.max.y - 0.5), pos2(title_rect.max.x, title_rect.max.y - 0.5)],
            Stroke::new(1.0, p.border_subtle),
        );
        // a title bar item: never the focus a dialog opens with, never a Tab stop
        let cross = Rect::from_min_size(
            pos2(title_rect.max.x - 8.0 - 24.0, title_rect.center().y - 12.0),
            Vec2::splat(24.0),
        );
        let c = ui.interact(cross, id.with("close"), Sense::CLICK);
        if c.hovered() {
            ui.painter().rect_filled(
                cross,
                CornerRadius::same(metrics::RADIUS),
                if c.is_pointer_button_down_on() { p.pressed } else { p.hover },
            );
        }
        icons::draw(
            ui.painter(),
            Icon::Close,
            Rect::from_center_size(cross.center(), Vec2::splat(metrics::GLYPH)),
            if c.hovered() { p.text } else { p.text_secondary },
        );
        if c.on_hover_text("Close").clicked() {
            close_now = true;
        }

        {
            let mut g = h.0.lock();
            cur_at = g.cur.len();
            g.stack.push(Drawn { name: name.to_owned(), rect });
        }
        // the audit names the dialog as its window is named, `Title###dlg.<name>`: what
        // `ui.audit id=dlg.<name>` and the contract's window names look for
        let window = format!("{title}###{name}");
        audit::surface(ctx, &window, || {
            audit::control(ctx, "Close", true, None, Some("Close"), cross);
            // the body scrolls; the window itself never does
            let body_rect = Rect::from_min_max(
                pos2(rect.min.x, title_rect.max.y),
                pos2(rect.max.x, (rect.max.y - FOOTER_H).max(title_rect.max.y + 24.0)),
            );
            let mut body_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt(id.with("body"))
                    .max_rect(body_rect)
                    .layout(Layout::top_down(Align::Min)),
            );
            body_ui.set_clip_rect(body_rect.intersect(ui.clip_rect()));
            let scroll = ScrollArea::vertical()
                .id_salt(id.with("scroll"))
                .max_height(body_rect.height())
                .auto_shrink([false, false])
                .show(&mut body_ui, |ui| {
                    Frame::NONE
                        .inner_margin(Margin::symmetric(16, 12))
                        .show(ui, |ui| {
                            ui.set_width((body_rect.width() - 32.0).max(1.0));
                            body(ui)
                        })
                        .inner
                });
            content_h = scroll.content_size.y;
            out = Some(scroll.inner);

            // the footer stays put right under the body
            let footer_rect = Rect::from_min_max(pos2(rect.min.x, body_rect.max.y), rect.max);
            ui.painter().line_segment(
                [
                    pos2(footer_rect.min.x, footer_rect.min.y + 0.5),
                    pos2(footer_rect.max.x, footer_rect.min.y + 0.5),
                ],
                Stroke::new(1.0, p.border_subtle),
            );
            let mut footer_ui =
                ui.new_child(UiBuilder::new().id_salt(id.with("footer")).max_rect(footer_rect));
            let mut f = Footer {
                ui: &mut footer_ui,
                rect: footer_rect,
                last_w: last_footer_w,
                x: footer_rect.max.x - 16.0 - last_footer_w,
                y: (footer_rect.min.y + (FOOTER_H - metrics::DIALOG_BUTTON_H) * 0.5).round(),
                w_cur: 0.0,
                danger: false,
                enter_ready,
                opening,
                buttons: Vec::new(),
                default_button: None,
                busy: false,
            };
            footer(&mut f);
            footer_state = (f.w_cur, f.danger, f.buttons, f.default_button, f.busy);
        });
        h.0.lock().stack.pop();
    });

    let (footer_w, danger, buttons, default_button, busy) = footer_state;
    mem.footer_w = footer_w;
    mem.danger = danger;
    mem.buttons = buttons;
    mem.default_button = default_button;
    if drag != Vec2::ZERO {
        mem.moved = true;
        mem.pos = Some(pos + drag);
    }
    // Fitted to what it holds while it opens. Afterwards it measures again only when what it
    // holds changed by itself - a note from a read that ended - while nobody works in it: a
    // change the author made, a focused field and a running job keep the height (the body
    // scrolls then).
    let now = ctx.input(|i| i.time);
    let touched = ctx.input(|i| {
        i.pointer.any_down()
            || i.pointer.any_released()
            || i.events.iter().any(|e| matches!(e, egui::Event::Key { .. } | egui::Event::Text(_)))
    });
    if touched {
        mem.touched = now;
    }
    let typing = ctx.memory(|m| m.focused()).is_some_and(|f| !mem.buttons.contains(&f));
    let quiet = !busy && !typing && now - mem.touched > QUIET_S;
    if mem.shown < MEASURE_FRAMES || (quiet && (content_h - mem.measured).abs() > 0.5) {
        mem.desired_h = TITLE_H + content_h + FOOTER_H + 1.0;
        mem.measured = content_h;
    } else if !quiet {
        mem.measured = content_h;
    }
    mem.shown += 1;
    mem.last_frame = frame;
    if close_now {
        *open = false;
    }
    let ppp = ctx.pixels_per_point();
    let r = drawn_rect;
    {
        // before the sheets its body drew: a sheet is on top of the dialog it belongs to
        let mut g = h.0.lock();
        let at = cur_at.min(g.cur.len());
        g.cur.insert(
            at,
            DialogInfo {
                id: name.to_owned(),
                title: title.to_owned(),
                rect_px: [r.min.x * ppp, r.min.y * ppp, r.max.x * ppp, r.max.y * ppp],
            },
        );
    }
    if *open {
        ctx.data_mut(|d| d.insert_temp(id, mem));
    } else {
        ctx.data_mut(|d| d.remove::<Mem>(id));
    }
    // the kit's questions raised inside this dialog are its sheets
    draw_sheet_of(ctx, name, r);
    out
}

fn headline(ui: &mut Ui, text_: &str) {
    if text_.is_empty() {
        return;
    }
    let p = palette(ui.ctx());
    ui.label(RichText::new(text_).font(Font::Strong.id()).color(p.text));
    ui.add_space(4.0);
}

fn list(ui: &mut Ui, items: &[String], total: usize, limit: usize) {
    let p = palette(ui.ctx());
    let shown = items.len().min(limit);
    for item in items.iter().take(shown) {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            text::label(ui, item, Font::Body, p.text, Clip::Middle);
        });
    }
    let all = total.max(items.len());
    if all > shown {
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            text::label(ui, &format!("and {} more", all - shown), Font::Body, p.text_muted, Clip::End);
        });
    }
}

fn draw_queued(ctx: &Context, q: &mut Queued, parent: Option<Drawn>) {
    let size = match &q.kind {
        QKind::Result(r) if r.rows.len() > 6 || !r.warnings.is_empty() => DialogSize::L,
        _ => DialogSize::M,
    };
    let mut open = q.open;
    let mut answer = None;
    let mut action = None;
    let kind = &q.kind;
    show_impl(
        ctx,
        &q.title,
        &q.name,
        size,
        parent,
        &mut open,
        |ui| {
            let p = palette(ui.ctx());
            match kind {
                QKind::Confirm(c) => {
                    headline(ui, &c.headline);
                    list(ui, &c.items, c.total, 10);
                    if !c.consequence.is_empty() || c.destructive {
                        ui.add_space(4.0);
                        let text_ = if c.consequence.is_empty() {
                            "Deleted for good - this cannot be undone."
                        } else {
                            c.consequence.as_str()
                        };
                        text::status(ui, Tone::Warning, text_);
                    }
                }
                QKind::Unsaved { headline: h, items } => {
                    headline(ui, h);
                    if items.len() > 1 {
                        list(ui, items, 0, 10);
                    }
                    ui.add_space(4.0);
                    text::status(ui, Tone::Muted, "Changes are lost if they are not saved.");
                }
                QKind::Notice { tone, text: t } => {
                    text::status(ui, *tone, t);
                }
                QKind::Result(r) => {
                    text::status(ui, r.tone, &r.headline);
                    if !r.rows.is_empty() {
                        ui.add_space(4.0);
                        let key_w = em(9.0);
                        for (key, value) in &r.rows {
                            let (row, _) = ui.allocate_exact_size(
                                Vec2::new(ui.available_width(), metrics::ROW),
                                Sense::hover(),
                            );
                            let font = Font::Body.id();
                            text::paint(
                                ui.painter(),
                                &font,
                                pos2(row.min.x, row.center().y),
                                Align2::LEFT_CENTER,
                                p.text_secondary,
                                key,
                                Some(key_w - 8.0),
                                Clip::End,
                            );
                            let vx = row.min.x + key_w;
                            text::paint(
                                ui.painter(),
                                &font,
                                pos2(vx, row.center().y),
                                Align2::LEFT_CENTER,
                                p.text,
                                value,
                                Some((row.max.x - vx).max(0.0)),
                                Clip::Middle,
                            );
                        }
                    }
                    if !r.left_out.is_empty() {
                        ui.add_space(4.0);
                        headline(ui, "Not shipped");
                        list(ui, &r.left_out, 0, 10);
                    }
                    if !r.warnings.is_empty() {
                        ui.add_space(4.0);
                        let shown = r.warnings.len().min(8);
                        for w in r.warnings.iter().take(shown) {
                            text::status(ui, Tone::Warning, w);
                        }
                        if r.warnings.len() > shown {
                            text::label(
                                ui,
                                &format!("and {} more warnings", r.warnings.len() - shown),
                                Font::Body,
                                p.text_muted,
                                Clip::End,
                            );
                        }
                    }
                }
            }
        },
        |f| match kind {
            QKind::Confirm(c) => {
                let label = if !c.action.is_empty() {
                    c.action.as_str()
                } else if c.destructive {
                    "Delete"
                } else {
                    "OK"
                };
                if f.button(
                    label,
                    if c.destructive { Kind::Danger } else { Kind::Primary },
                    !c.destructive,
                    true,
                    "",
                ) {
                    answer = Some(Answer::Yes);
                }
                if f.button("Cancel", Kind::Secondary, c.destructive, true, "") {
                    answer = Some(Answer::Cancel);
                }
            }
            QKind::Unsaved { items, .. } => {
                if f.button(if items.len() > 1 { "Save All" } else { "Save" }, Kind::Primary, true, true, "")
                {
                    answer = Some(Answer::Yes);
                }
                if f.button("Don't Save", Kind::Secondary, false, true, "") {
                    answer = Some(Answer::No);
                }
                if f.button("Cancel", Kind::Secondary, false, true, "") {
                    answer = Some(Answer::Cancel);
                }
            }
            QKind::Notice { .. } => {
                if f.button("OK", Kind::Primary, true, true, "") {
                    answer = Some(Answer::Yes);
                }
            }
            QKind::Result(r) => {
                if let Some(a) = &r.action
                    && f.button(&a.label, Kind::Secondary, false, true, "")
                {
                    action = Some(a.run.clone());
                }
                if f.button("OK", Kind::Primary, true, true, "") {
                    answer = Some(Answer::Yes);
                }
            }
        },
    );
    if let Some(run) = action {
        handle(ctx).0.lock().actions.push(run);
    }
    match answer {
        Some(a) => {
            q.answer = Some(a);
            q.open = false;
        }
        None if !open => {
            q.answer = Some(Answer::Cancel);
            q.open = false;
        }
        None => {}
    }
}

/// Draws the first open queued question whose parent is `parent` (a sheet), or the first one
/// without a parent when `parent` is `None`.
fn draw_one(ctx: &Context, parent: Option<(&str, Rect)>) {
    let h = handle(ctx);
    let taken = {
        let mut s = h.0.lock();
        let at =
            s.queue.iter().position(|q| q.answer.is_none() && q.parent.as_deref() == parent.map(|(n, _)| n));
        at.map(|i| (i, s.queue.remove(i)))
    };
    let Some((i, mut q)) = taken else { return };
    draw_queued(ctx, &mut q, parent.map(|(name, rect)| Drawn { name: name.to_owned(), rect }));
    let mut s = h.0.lock();
    let at = i.min(s.queue.len());
    s.queue.insert(at, q);
}

fn draw_sheet_of(ctx: &Context, name: &str, rect: Rect) {
    draw_one(ctx, Some((name, rect)));
}

/// Runs the answers of the questions answered, outside every window.
fn finish_answered(ctx: &Context) {
    let h = handle(ctx);
    let (done, actions) = {
        let mut s = h.0.lock();
        let mut done = Vec::new();
        let mut i = 0;
        while i < s.queue.len() {
            if s.queue.get(i).is_some_and(|q| q.answer.is_some()) {
                let q = s.queue.remove(i);
                done.push(q);
            } else {
                i += 1;
            }
        }
        (done, std::mem::take(&mut s.actions))
    };
    for run in actions {
        run();
    }
    for q in done {
        if let (Some(then), Some(answer)) = (q.then, q.answer) {
            then(answer);
        }
    }
}

/// Draws the kit's queued questions at the end of the frame, nested above any modal open, and
/// runs the answers given. Call it once per frame, last.
pub fn draw_overlays(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    finish_answered(&ctx);
    {
        // a sheet whose dialog has gone becomes a dialog of its own
        let h = handle(&ctx);
        let mut s = h.0.lock();
        let Shared { queue, cur, .. } = &mut *s;
        for q in queue.iter_mut() {
            if q.parent.as_ref().is_some_and(|p| !cur.iter().any(|d| &d.id == p)) {
                q.parent = None;
            }
        }
    }
    draw_one(&ctx, None);
    finish_answered(&ctx);
    let h = handle(&ctx);
    let mut s = h.0.lock();
    s.last = std::mem::take(&mut s.cur);
    s.stack.clear();
}

fn queue(ctx: &Context, kind: QKind, title: String, name: String, then: Option<Then>) {
    let h = handle(ctx);
    let mut s = h.0.lock();
    // a question raised inside a dialog is a sheet of it
    let parent = s.stack.last().map(|d| d.name.clone());
    s.queue.push(Queued { kind, title, name, parent, open: true, answer: None, then });
    drop(s);
    ctx.request_repaint();
}

fn or<'s>(value: &'s str, fallback: &'s str) -> &'s str {
    if value.is_empty() { fallback } else { value }
}

/// What a note goes into the log as: an error, a warning, or information.
fn logged(tone: Tone, what: &str, text_: &str) -> Finding {
    let severity = match tone {
        Tone::Error => Severity::Error,
        Tone::Warning => Severity::Warning,
        _ => Severity::Note,
    };
    let sep = if !what.is_empty() && !text_.is_empty() { ": " } else { "" };
    Finding::new(severity, format!("{what}{sep}{text_}"))
}

/// Held while a caller who has decided runs something of the window (an MCP caller's row of the
/// command table): every question raised through this context meanwhile is that caller's to
/// answer, whatever asker the code that raises it names. Dropped, the scope it was opened in is
/// in force again (none: each question has its own asker).
#[must_use]
pub struct CallerScope {
    ctx: Context,
    outer: Option<Asker>,
}

impl Drop for CallerScope {
    fn drop(&mut self) {
        handle(&self.ctx).0.lock().caller = self.outer;
    }
}

/// Opens a [`CallerScope`] on the UI thread, for the length of a synchronous run.
pub fn answer_as(ctx: &Context, caller: Asker) -> CallerScope {
    let outer = handle(ctx).0.lock().caller.replace(caller);
    CallerScope { ctx: ctx.clone(), outer }
}

/// The asker in force: the caller's while its scope is held.
pub(crate) fn who(ctx: &Context, asker: Asker) -> Asker {
    handle(ctx).0.lock().caller.unwrap_or(asker)
}

/// Asks to confirm an action. `Mcp` runs `then(Yes)` before returning, `Unattended` runs
/// `then(No)`; both return the finding to log. `Ui` queues the dialog and returns `None`.
pub fn confirm(
    ctx: &Context,
    asker: Asker,
    spec: ConfirmSpec,
    then: impl FnOnce(Answer) + Send + 'static,
) -> Option<Finding> {
    let what = or(&spec.headline, or(&spec.title, "Confirm")).to_owned();
    match who(ctx, asker) {
        Asker::Mcp => {
            then(Answer::Yes);
            Some(Finding::info(format!("[mcp] {what} -> Yes")))
        }
        Asker::Unattended => {
            then(Answer::No);
            Some(Finding::warning(format!("[dlg suppressed] {what}  -> auto-answer: No")))
        }
        Asker::Ui => {
            let title = or(&spec.title, "Confirm").to_owned();
            let name = full_name(or(&spec.id, "confirm"));
            queue(ctx, QKind::Confirm(spec), title, name, Some(Box::new(then)));
            None
        }
    }
}

/// Asks to save before something is lost: Save (Yes), Don't Save (No), Cancel. `what` names
/// the dirty documents. `Mcp` answers Cancel (a command decides with its `save` / `discard`
/// argument before it asks), `Unattended` No; both return the finding to log.
pub fn unsaved_prompt(
    ctx: &Context,
    asker: Asker,
    id: &str,
    what: &[String],
    then: impl FnOnce(Answer) + Send + 'static,
) -> Option<Finding> {
    let joined = what.join(", ");
    match who(ctx, asker) {
        Asker::Mcp => {
            then(Answer::Cancel);
            Some(Finding::info(format!("[mcp] unsaved changes ({joined}) -> Cancel")))
        }
        Asker::Unattended => {
            then(Answer::No);
            Some(Finding::warning(format!("[dlg suppressed] Unsaved changes: {joined}  -> auto-answer: No")))
        }
        Asker::Ui => {
            let headline = match what {
                [one] => format!("Save changes to {one}?"),
                _ => "Save changes to these items?".to_owned(),
            };
            let kind = QKind::Unsaved { headline, items: what.to_vec() };
            queue(
                ctx,
                kind,
                "Unsaved Changes".to_owned(),
                full_name(or(id, "unsaved")),
                Some(Box::new(then)),
            );
            None
        }
    }
}

/// A note for the author: an error, a warning, information. Returns the finding to log; only `Ui`
/// opens it.
pub fn notice(ctx: &Context, asker: Asker, tone: Tone, title: &str, text_: &str) -> Option<Finding> {
    let line = logged(tone, title, text_);
    if who(ctx, asker) == Asker::Ui {
        let fallback = match tone {
            Tone::Error => "Error",
            Tone::Warning => "Warning",
            _ => "Information",
        };
        queue(
            ctx,
            QKind::Notice { tone, text: text_.to_owned() },
            or(title, fallback).to_owned(),
            "dlg.notice".to_owned(),
            None,
        );
    }
    Some(line)
}

/// The result of a run started from the UI: a toned headline, rows, Not Shipped, up to 8
/// warnings, the action and OK. Returns the findings to log (the headline at the tone of the
/// result, then each warning); only `Ui` opens it.
pub fn result(ctx: &Context, asker: Asker, spec: ResultSpec) -> Vec<Finding> {
    let mut lines = vec![logged(spec.tone, &spec.title, &spec.headline)];
    lines.extend(spec.warnings.iter().map(Finding::warning));
    if who(ctx, asker) == Asker::Ui {
        let title = or(&spec.title, "Result").to_owned();
        let name = full_name(or(&spec.id, "result"));
        queue(ctx, QKind::Result(spec), title, name, None);
    }
    lines
}

/// The dialogs the last frame drew, topmost last.
#[must_use]
pub fn open_dialogs(ctx: &Context) -> Vec<DialogInfo> {
    handle(ctx).0.lock().last.clone()
}

/// The dialog on top, if any.
#[must_use]
pub fn modal_on_top(ctx: &Context) -> Option<DialogInfo> {
    handle(ctx).0.lock().last.last().cloned()
}

/// Closes a dialog as Esc would (a question is answered Cancel). `id` is `dlg.<name>` or the
/// name alone. Returns whether such a dialog was open.
pub fn close_dialog(ctx: &Context, id: &str) -> bool {
    if id.is_empty() {
        return false;
    }
    let name = full_name(id);
    let h = handle(ctx);
    let mut s = h.0.lock();
    let mut found = false;
    for q in s.queue.iter_mut().filter(|q| q.answer.is_none() && q.name == name) {
        q.answer = Some(Answer::Cancel);
        q.open = false;
        found = true;
    }
    if s.last.iter().any(|d| d.id == name) {
        if !s.close_requests.contains(&name) {
            s.close_requests.push(name);
        }
        found = true;
    }
    drop(s);
    if found {
        ctx.request_repaint();
    }
    found
}

#[cfg(test)]
mod tests {
    use super::{Answer, Asker, ConfirmSpec, DialogSize, ResultSpec, answer_as, full_name, handle, logged};
    use crate::theme::Tone;
    use gz_base::Finding;

    /// Raises each kind of question as `asker` and returns what each answered: confirm, unsaved;
    /// with the number of dialogs queued by the four calls.
    fn ask_all(ctx: &egui::Context, asker: Asker) -> (Option<Answer>, Option<Answer>, usize) {
        let before = handle(ctx).0.lock().queue.len();
        let (tx, rx) = std::sync::mpsc::channel();
        let tx2 = tx.clone();
        super::confirm(
            ctx,
            asker,
            ConfirmSpec { title: "Delete".into(), ..ConfirmSpec::default() },
            move |a| {
                let _ = tx.send(a);
            },
        );
        let confirmed = rx.try_recv().ok();
        super::unsaved_prompt(ctx, asker, "unsaved", &["quest.nqasset".to_owned()], move |a| {
            let _ = tx2.send(a);
        });
        let unsaved = rx.try_recv().ok();
        super::notice(ctx, asker, Tone::Warning, "Note", "text");
        super::result(ctx, asker, ResultSpec::new("build_result", "Build", Tone::Info, "Built."));
        (confirmed, unsaved, handle(ctx).0.lock().queue.len() - before)
    }

    #[test]
    fn a_callers_scope_answers_every_question_and_ends_with_itself() {
        let ctx = egui::Context::default();
        {
            let _scope = answer_as(&ctx, Asker::Mcp);
            // whatever asker the code names, the MCP caller answers: Yes, Cancel, nothing opens
            for asker in [Asker::Ui, Asker::Mcp, Asker::Unattended] {
                assert_eq!(ask_all(&ctx, asker), (Some(Answer::Yes), Some(Answer::Cancel), 0), "{asker:?}");
            }
            {
                // a scope inside a scope gives the outer one back when it ends
                let _inner = answer_as(&ctx, Asker::Unattended);
                assert_eq!(ask_all(&ctx, Asker::Ui), (Some(Answer::No), Some(Answer::No), 0));
            }
            assert_eq!(ask_all(&ctx, Asker::Ui), (Some(Answer::Yes), Some(Answer::Cancel), 0));
        }
        // each asker is itself again: a person gets four dialogs and no answer yet
        assert_eq!(ask_all(&ctx, Asker::Ui), (None, None, 4));
        assert_eq!(ask_all(&ctx, Asker::Mcp), (Some(Answer::Yes), Some(Answer::Cancel), 0));
        assert_eq!(ask_all(&ctx, Asker::Unattended), (Some(Answer::No), Some(Answer::No), 0));
    }

    #[test]
    fn names_carry_the_prefix_once() {
        assert_eq!(full_name("delete_project"), "dlg.delete_project");
        assert_eq!(full_name("dlg.delete_project"), "dlg.delete_project");
    }

    #[test]
    fn sizes() {
        assert_eq!(DialogSize::M.width(1707.0), 560.0);
        assert_eq!(DialogSize::XL.width(1000.0), 900.0);
        assert_eq!(DialogSize::XL.width(2000.0), 1040.0);
    }

    #[test]
    fn a_note_is_logged_at_the_level_of_its_tone() {
        assert_eq!(logged(Tone::Error, "Build", "failed"), Finding::error("Build: failed"));
        assert_eq!(logged(Tone::Warning, "", "careful"), Finding::warning("careful"));
        assert_eq!(logged(Tone::Info, "Done", ""), Finding::info("Done"));
    }

    #[test]
    fn mcp_confirms_at_once_and_unattended_declines() {
        let ctx = egui::Context::default();
        let spec = ConfirmSpec {
            title: "Delete Project".into(),
            headline: "Delete Overnight Delivery?".into(),
            destructive: true,
            ..ConfirmSpec::default()
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let tx2 = tx.clone();
        let line = super::confirm(&ctx, Asker::Mcp, spec.clone(), move |a| {
            let _ = tx.send(a);
        });
        assert_eq!(rx.try_recv().ok(), Some(Answer::Yes));
        assert_eq!(line, Some(Finding::info("[mcp] Delete Overnight Delivery? -> Yes")));
        let line = super::confirm(&ctx, Asker::Unattended, spec, move |a| {
            let _ = tx2.send(a);
        });
        assert_eq!(rx.try_recv().ok(), Some(Answer::No));
        assert_eq!(
            line,
            Some(Finding::warning("[dlg suppressed] Delete Overnight Delivery?  -> auto-answer: No"))
        );
    }
}
