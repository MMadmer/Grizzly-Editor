//! Drag and drop between surfaces (docs/UI_DESIGN.md, section 4): typed payloads and drop zones.
//!
//! A surface that lets something be dragged out of it sets a [`Payload`] ([`start`]); a surface
//! that takes something declares a zone over a rectangle ([`zone`]) with what it accepts. A
//! zone lights up while an acceptable payload is over it and answers the payload when it is let
//! go there. Files from Explorer land the same way: Windows says that files landed, not where, so
//! the drop waits for the pointer's next position (the system moves it over the window as the
//! drag ends) and takes the last one known after half a second; a drop with no position at all
//! goes to the zone that catches everything ([`Zone::catch_all`]). Zones are asked in the order
//! they are drawn, inner ones first: the first one under the point takes it.
//!
//! Call [`begin`] once a frame before the first zone (the shell does).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Context, CornerRadius, DragAndDrop, Event, Id, Pos2, Rect, Response, Stroke, StrokeKind, Ui};

use crate::theme::{Palette, palette};

/// A file of the project or the game the Content Browser drags: its project-relative path, the
/// kind of asset it is, and where it is on the disk when it is a file there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub path: String,
    /// `quest`, `texture`, `model`, `folder`, ... (`gz-project`'s asset kinds, `folder` for one).
    pub kind: String,
    pub file: Option<PathBuf>,
}

/// What is being dragged.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    /// Entries of the Content Browser.
    Assets(Vec<Asset>),
    /// Files on the disk (from Explorer, or a surface's own).
    Files(Vec<PathBuf>),
    /// A row of a surface, by the surface's own key (a layer of a screen, a node of a graph).
    Row {
        surface: String,
        key: String,
    },
    Text(String),
}

impl Payload {
    /// The files on the disk the payload carries: the files of assets, files as they are.
    #[must_use]
    pub fn files(&self) -> Vec<PathBuf> {
        match self {
            Payload::Assets(list) => {
                list.iter().filter(|a| a.kind != "folder").filter_map(|a| a.file.clone()).collect()
            }
            Payload::Files(list) => list.clone(),
            _ => Vec::new(),
        }
    }
}

/// Starts dragging `payload` from `resp` (call it while `resp.dragged()`).
pub fn start(resp: &Response, payload: Payload) {
    if resp.drag_started() || resp.dragged() {
        DragAndDrop::set_payload(&resp.ctx, payload);
    }
}

/// What is being dragged now, when it is one of ours.
#[must_use]
pub fn dragged(ctx: &Context) -> Option<Arc<Payload>> {
    DragAndDrop::payload::<Payload>(ctx)
}

/// How long a drop from Explorer waits for the pointer to say where it landed.
const LANDING_WAIT: Duration = Duration::from_millis(500);
/// `Landed::frame` of a drop that lands in the frame that sees it first.
const FRESH: u64 = u64::MAX;

#[derive(Clone)]
struct Pending {
    files: Vec<PathBuf>,
    since: Instant,
    at: Option<Pos2>,
    frame: u64,
}

#[derive(Clone)]
struct Landed {
    files: Vec<PathBuf>,
    pos: Option<Pos2>,
    frame: u64,
}

#[derive(Clone, Default)]
struct Board {
    frame: u64,
    begun: bool,
    pending: Option<Pending>,
    landed: Option<Landed>,
    taken: bool,
    lit: bool,
}

fn board_id() -> Id {
    Id::unique("gz-ui.drop.board")
}

fn with_board<R>(ctx: &Context, f: impl FnOnce(&mut Board) -> R) -> R {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default::<Board>(board_id())))
}

/// Starts this frame's drops: what Explorer delivered waits for its point, what landed last
/// frame and nobody took fell where nothing listens.
pub fn begin(ctx: &Context) {
    let frame = ctx.cumulative_frame_nr();
    let (dropped, moved, latest) = ctx.input(|i| {
        let files: Vec<PathBuf> = i
            .raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .filter(|p| !p.as_os_str().is_empty())
            .collect();
        let moved = i
            .raw
            .events
            .iter()
            .rev()
            .find_map(|e| if let Event::PointerMoved(p) = e { Some(*p) } else { None });
        (files, moved, i.pointer.latest_pos())
    });
    let wait = with_board(ctx, |b| {
        if b.begun && b.frame == frame {
            return false;
        }
        b.begun = true;
        b.frame = frame;
        b.taken = false;
        b.lit = false;
        if let Some(l) = &mut b.landed {
            if l.frame == FRESH {
                l.frame = frame;
            } else if l.frame < frame {
                b.landed = None;
            }
        }
        if !dropped.is_empty() {
            b.pending = Some(Pending { files: dropped, since: Instant::now(), at: latest, frame });
        }
        let Some(p) = b.pending.take() else { return false };
        if let Some(pos) = moved.filter(|_| frame > p.frame) {
            b.landed = Some(Landed { files: p.files, pos: Some(pos), frame });
            false
        } else if p.since.elapsed() >= LANDING_WAIT {
            b.landed = Some(Landed { files: p.files, pos: latest.or(p.at), frame });
            false
        } else {
            b.pending = Some(p);
            true
        }
    });
    if wait {
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

/// Lands files at a point of the window (pt) the way a drop from Explorer lands: the zones of the
/// next frame are asked for them. For MCP and tests.
pub fn simulate_files(ctx: &Context, files: Vec<PathBuf>, pos: Pos2) {
    with_board(ctx, |b| {
        b.pending = None;
        b.landed = Some(Landed { files, pos: Some(pos), frame: FRESH });
    });
    ctx.request_repaint();
}

fn light(ui: &Ui, rect: Rect, p: &Palette) {
    ui.painter().rect(
        rect.shrink(1.0),
        CornerRadius::same(4),
        Palette::alpha(p.accent, 0.14),
        Stroke::new(2.0, p.accent_bright),
        StrokeKind::Inside,
    );
}

/// A place a payload can be dropped on.
#[must_use = "a zone does nothing until it is asked: .take(ui)"]
pub struct Zone<'a> {
    name: &'a str,
    rect: Rect,
    accepts: &'a dyn Fn(&Payload) -> bool,
    catch_all: bool,
}

impl<'a> Zone<'a> {
    /// A zone over `rect` (pt) that takes what `accepts` says yes to; `name` is what the audit
    /// calls it.
    pub fn new(name: &'a str, rect: Rect, accepts: &'a dyn Fn(&Payload) -> bool) -> Self {
        Self { name, rect, accepts, catch_all: false }
    }

    /// The outermost zone of a surface: it also takes files nobody knows the point of.
    pub fn catch_all(mut self) -> Self {
        self.catch_all = true;
        self
    }

    /// Asks the zone, after what it covers is drawn: the payload let go on it this frame.
    pub fn take(self, ui: &Ui) -> Option<Payload> {
        let ctx = ui.ctx().clone();
        if !self.rect.is_positive() {
            return None;
        }
        let shown = self.rect.intersect(ui.clip_rect());
        let over = ctx.pointer_latest_pos().is_some_and(|at| shown.contains(at));
        if over && let Some(payload) = dragged(&ctx).filter(|p| (self.accepts)(p)) {
            if ctx.input(|i| i.pointer.any_released()) {
                DragAndDrop::clear_payload(&ctx);
                with_board(&ctx, |b| b.lit = true);
                return Some((*payload).clone());
            }
            if with_board(&ctx, |b| !std::mem::replace(&mut b.lit, true)) {
                light(ui, self.rect, &palette(&ctx));
                crate::audit::control(&ctx, self.name, true, None, Some("Drop it here"), self.rect);
            }
        }
        let accepts = self.accepts;
        let catch_all = self.catch_all;
        with_board(&ctx, |b| {
            if b.taken {
                return None;
            }
            let hit = b.landed.as_ref().is_some_and(|l| match l.pos {
                Some(at) => shown.contains(at),
                None => catch_all,
            });
            if !hit {
                return None;
            }
            let files = b.landed.as_ref().map(|l| Payload::Files(l.files.clone()))?;
            if !accepts(&files) {
                return None;
            }
            b.taken = true;
            b.landed = None;
            Some(files)
        })
    }
}

/// A zone that takes files: from Explorer, or the files of dragged assets.
#[must_use]
pub fn accepts_files(p: &Payload) -> bool {
    !p.files().is_empty()
}
