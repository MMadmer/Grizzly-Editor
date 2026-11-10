//! Files dropped on a surface (`XrUIFileDrop.cpp`), as [`local_file_drop_zone`]: from Explorer
//! (egui's dropped files - only where they land counts, nothing lights up on their way) and from
//! the Content Browser's tiles (an egui drag-and-drop payload: a tile lights up the zone under
//! it). Zones are asked in the order they are drawn, inner ones first: the first under the point
//! takes the files.
//!
//! The Content Browser's payload is its `DragPayload` (the dragged entries that are files on the
//! disk); a `String`, a `PathBuf`, a `Vec<String>` or a `Vec<PathBuf>` is taken too.
//!
//! Windows tells a window that files landed, not where: the drop waits for the pointer's next
//! position (the system moves it over the window as the drag ends) and takes the last one known
//! after half a second; a drop with no position at all goes to the zone that catches everything.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Context, CornerRadius, DragAndDrop, Event, Id, Pos2, Rect, Stroke, StrokeKind, Ui};
use gz_ui::{Palette, palette};

/// How long a drop from Explorer waits for the pointer to say where it landed.
const LANDING_WAIT: Duration = Duration::from_millis(500);
/// `Landed::frame` of a drop that lands in the frame that sees it first.
const FRESH: u64 = u64::MAX;

#[derive(Clone)]
struct Pending {
    files: Vec<String>,
    since: Instant,
    at: Option<Pos2>,
    frame: u64,
}

#[derive(Clone)]
struct Landed {
    files: Vec<String>,
    /// pt; `None`: nobody knows where.
    pos: Option<Pos2>,
    frame: u64,
}

#[derive(Clone, Default)]
struct Board {
    frame: u64,
    begun: bool,
    pending: Option<Pending>,
    landed: Option<Landed>,
    /// A zone took this frame's files.
    taken: bool,
    /// A zone lit up under a dragged tile this frame.
    lit: bool,
}

fn board_id() -> Id {
    Id::unique("gz-ui-pda.drops")
}

fn with_board<R>(ctx: &Context, f: impl FnOnce(&mut Board) -> R) -> R {
    ctx.data_mut(|d| f(d.get_temp_mut_or_default::<Board>(board_id())))
}

/// Starts this frame's drops: what Explorer delivered waits for its point, what landed last
/// frame and nobody took fell where nothing listens. A surface with zones calls it once a frame
/// before its first zone.
pub fn begin_drops(ctx: &Context) {
    let frame = ctx.cumulative_frame_nr();
    let (dropped, moved, latest) = ctx.input(|i| {
        let files: Vec<String> = i.raw.dropped_files.iter().map(|f| f.path().display().to_string()).filter(|p| !p.is_empty()).collect();
        let moved = i.raw.events.iter().rev().find_map(|e| if let Event::PointerMoved(p) = e { Some(*p) } else { None });
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
        // a move of the frame the files came in may be from before they landed
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

/// Lands files at a point of the window (pt) the way a drop from Explorer lands: the zones of
/// the next frame drawn are asked for them.
pub fn simulate_drop(ctx: &Context, files: Vec<String>, pos: Pos2) {
    with_board(ctx, |b| {
        b.pending = None;
        b.landed = Some(Landed { files, pos: Some(pos), frame: FRESH });
    });
    ctx.request_repaint();
}

/// The paths a dragged tile carries, if one is being dragged.
fn payload_paths(ctx: &Context) -> Option<Vec<String>> {
    // the Content Browser's tiles: the files they carry that are files on the disk (a game file
    // inside an archive is not one, and a folder is not a layer)
    if let Some(p) = DragAndDrop::payload::<gz_ui_content::DragPayload>(ctx) {
        return Some(p.entries.iter().filter(|e| !e.folder).filter_map(|e| e.file.as_ref().map(|f| f.display().to_string())).collect());
    }
    if let Some(s) = DragAndDrop::payload::<String>(ctx) {
        return Some(vec![(*s).clone()]);
    }
    if let Some(p) = DragAndDrop::payload::<PathBuf>(ctx) {
        return Some(vec![p.display().to_string()]);
    }
    if let Some(v) = DragAndDrop::payload::<Vec<String>>(ctx) {
        return Some((*v).clone());
    }
    DragAndDrop::payload::<Vec<PathBuf>>(ctx).map(|v| v.iter().map(|p| p.display().to_string()).collect())
}

fn light(ui: &Ui, rect: Rect, p: &Palette) {
    let painter = ui.painter();
    painter.rect(rect.shrink(1.0), CornerRadius::same(4), Palette::alpha(p.accent, 0.14), Stroke::new(2.0, p.accent_bright), StrokeKind::Inside);
}

/// A place files can be dropped on: `rect` in pt. Returns the files that landed on it this frame
/// - absolute or project-relative paths. Call it after what it covers is drawn, inner zones
/// before outer ones; `catch_all` is the outermost zone of a surface, which also takes a drop
/// nobody knows the point of.
pub fn local_file_drop_zone(ui: &Ui, id_salt: &str, rect: Rect, catch_all: bool) -> Option<Vec<String>> {
    let ctx = ui.ctx().clone();
    if !rect.is_positive() {
        return None;
    }
    let shown = rect.intersect(ui.clip_rect());
    let pointer = ctx.pointer_latest_pos();
    let over = pointer.is_some_and(|at| shown.contains(at));
    // a tile dragged out of the Content Browser
    if over && let Some(paths) = payload_paths(&ctx).filter(|p| !p.is_empty()) {
        let released = ctx.input(|i| i.pointer.any_released());
        if released {
            DragAndDrop::clear_payload(&ctx);
            with_board(&ctx, |b| b.lit = true);
            return Some(paths);
        }
        let first = with_board(&ctx, |b| !std::mem::replace(&mut b.lit, true));
        if first {
            light(ui, rect, &palette(&ctx));
            gz_ui::audit::control(&ctx, id_salt, true, None, Some("Drop it here"), rect);
        }
    }
    // files from Explorer
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
        b.taken = true;
        b.landed.take().map(|l| l.files)
    })
}
