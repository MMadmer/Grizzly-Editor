//! `ui_scroll` for surfaces the shell does not draw itself: a panel, a drawer's body, a dialog.
//!
//! A surface that scrolls reports where it stands each frame with [`report_scroll`] and takes a
//! request with [`take_scroll`] before it draws its scroll area. Both go through the context's
//! memory, so neither needs the shell.

use egui::{Context, Id};
use rustc_hash::FxHashMap;

/// Where `ui_scroll` asks a surface to go.
#[derive(Clone, Debug, PartialEq)]
pub enum ScrollTo {
    Top,
    Bottom,
    /// pt from the top.
    Offset(f32),
    /// A named item the surface knows (`ScrollTarget` of the kit's lists and sections).
    Item(String),
}

impl ScrollTo {
    /// `top`, `bottom`, a number of pt, or `item:<name>`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "top" => Some(ScrollTo::Top),
            "bottom" => Some(ScrollTo::Bottom),
            _ => match text.strip_prefix("item:") {
                Some(name) if !name.is_empty() => Some(ScrollTo::Item(name.to_owned())),
                Some(_) => None,
                None => text.trim().parse::<f32>().ok().filter(|v| v.is_finite() && *v >= 0.0).map(ScrollTo::Offset),
            },
        }
    }
}

#[derive(Clone, Default)]
struct Board {
    /// window -> (frame, scroll y, max scroll y), pt.
    reports: FxHashMap<String, (u64, f32, f32)>,
    requests: FxHashMap<String, ScrollTo>,
}

fn board_id() -> Id {
    Id::unique("gz-app.scroll_board")
}

/// A surface tells where its scroll stands this frame (pt).
pub fn report_scroll(ctx: &Context, window: &str, y: f32, max_y: f32) {
    let frame = ctx.cumulative_frame_nr();
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Board>(board_id()).reports.insert(window.to_owned(), (frame, y, max_y)));
}

/// A surface takes the scroll asked of it, once.
#[must_use]
pub fn take_scroll(ctx: &Context, window: &str) -> Option<ScrollTo> {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Board>(board_id()).requests.remove(window))
}

pub(crate) fn request_scroll(ctx: &Context, window: &str, to: ScrollTo) {
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Board>(board_id()).requests.insert(window.to_owned(), to));
}

/// Where a surface's scroll stood, when it reported in the last two frames.
pub(crate) fn last_scroll(ctx: &Context, window: &str) -> Option<(f32, f32)> {
    let frame = ctx.cumulative_frame_nr();
    let found = ctx.data_mut(|d| d.get_temp_mut_or_default::<Board>(board_id()).reports.get(window).copied());
    found.filter(|(f, _, _)| f + 2 >= frame).map(|(_, y, m)| (y, m))
}

#[cfg(test)]
mod tests {
    use super::ScrollTo;

    #[test]
    fn targets() {
        assert_eq!(ScrollTo::parse("top"), Some(ScrollTo::Top));
        assert_eq!(ScrollTo::parse("120"), Some(ScrollTo::Offset(120.0)));
        assert_eq!(ScrollTo::parse("item:Snap"), Some(ScrollTo::Item("Snap".into())));
        assert_eq!(ScrollTo::parse("item:"), None);
        assert_eq!(ScrollTo::parse("sideways"), None);
        assert_eq!(ScrollTo::parse("-4"), None);
    }
}
