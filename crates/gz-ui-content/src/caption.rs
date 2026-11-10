//! A tile's caption (`DrawTileCaption`), as a pure function of the name and a measure of text
//! widths.
//!
//! Up to `max_lines` lines. A line breaks where a name breaks for a reader: after a space, `_`,
//! `-`, `.` or `,`, and between the words of a CamelCase name (`NewBehaviour` -> `New` /
//! `Behaviour`); mid-word only when one word is wider than the tile. Never right before the
//! extension: `bytil_wnd.dds` goes as `bytil_` / `wnd.dds`. A last line that does not fit keeps
//! the extension and loses the middle of the name.

/// The ellipsis a cut line carries.
pub const ELLIPSIS: &str = "\u{2026}";

/// What a caption shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Caption {
    pub lines: Vec<String>,
    /// The last line lost part of the name.
    pub truncated: bool,
}

fn is_lower(c: char) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit()
}

/// The end (byte) of the longest prefix of `text[start..end]` that is at most `room` wide.
fn fit_end(text: &str, start: usize, end: usize, room: f32, measure: &dyn Fn(&str) -> f32) -> usize {
    let Some(slice) = text.get(start..end) else { return start };
    let bounds: Vec<usize> = slice.char_indices().map(|(i, _)| start + i).skip(1).chain(std::iter::once(end)).collect();
    // the number of characters kept: a binary search on the widths, which only grow
    let (mut lo, mut hi) = (0usize, bounds.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let at = bounds.get(mid - 1).copied().unwrap_or(end);
        if measure(text.get(start..at).unwrap_or("")) <= room {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    if lo == 0 { start } else { bounds.get(lo - 1).copied().unwrap_or(end) }
}

/// The lines of `text` in `width`. `has_ext`: the name carries an extension that must stay whole.
#[must_use]
pub fn caption(text: &str, has_ext: bool, max_lines: usize, width: f32, measure: &dyn Fn(&str) -> f32) -> Caption {
    let mut out = Caption::default();
    let end = text.len();
    // where the extension starts: no line may begin at it or inside it
    let ext = if has_ext { text.rfind('.').filter(|&i| i > 0) } else { None };
    let ellipsis_w = measure(ELLIPSIS);
    let mut start = 0usize;
    for line in 0..max_lines {
        while text.get(start..).is_some_and(|r| r.starts_with(' ')) {
            start += 1;
        }
        if start >= end {
            break;
        }
        let rest = text.get(start..end).unwrap_or("");
        if measure(rest) <= width {
            out.lines.push(rest.to_owned());
            break;
        }
        if line + 1 == max_lines {
            // the rest does not fit: the extension stays whole, the name gives up its middle
            let keep_ext = ext.is_some_and(|e| e > start);
            let tail_w = match ext {
                Some(e) if keep_ext => measure(text.get(e..end).unwrap_or("")),
                _ => 0.0,
            };
            let room = width - ellipsis_w - tail_w;
            let head_end = if keep_ext { ext.unwrap_or(end) } else { end };
            let fit = if room > 0.0 { fit_end(text, start, head_end, room, measure) } else { start };
            let mut shown = String::with_capacity(rest.len() + 3);
            match ext {
                Some(e) if keep_ext && room > 0.0 => {
                    shown.push_str(text.get(start..fit).unwrap_or(""));
                    shown.push_str(ELLIPSIS);
                    shown.push_str(text.get(e..end).unwrap_or(""));
                }
                _ if keep_ext => {
                    // not even the extension fits: the end goes instead
                    let fit = fit_end(text, start, end, (width - ellipsis_w).max(0.0), measure);
                    shown.push_str(text.get(start..fit).unwrap_or(""));
                    shown.push_str(ELLIPSIS);
                }
                _ => {
                    shown.push_str(text.get(start..fit).unwrap_or(""));
                    shown.push_str(ELLIPSIS);
                }
            }
            out.lines.push(shown);
            out.truncated = true;
            break;
        }
        // what fits, then the last place a reader would break it
        let mut fit = fit_end(text, start, end, width, measure);
        if fit <= start {
            fit = text.get(start..).and_then(|r| r.chars().next()).map_or(end, |c| start + c.len_utf8());
        }
        let mut brk: Option<usize> = None;
        let mut chars = text.get(start..fit).unwrap_or("").char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            let next = start + i + c.len_utf8();
            let next_char = text.get(next..).and_then(|r| r.chars().next());
            let breaks = " _-.,".contains(c) || (is_lower(c) && next_char.is_some_and(|n| n.is_ascii_uppercase()));
            if next <= fit && next < end && ext.is_none_or(|e| next < e) && breaks {
                brk = Some(next);
            }
        }
        let mut cut = brk.unwrap_or(fit);
        // a word wider than the tile is cut where it stops fitting - still short of its extension
        if let Some(e) = ext
            && cut >= e
            && brk.is_none()
        {
            let back = text.get(..e).and_then(|h| h.char_indices().next_back()).map_or(0, |(i, _)| i);
            if back > start {
                cut = back;
            }
        }
        out.lines.push(text.get(start..cut).unwrap_or("").to_owned());
        start = cut;
    }
    out
}

/// A list row's one line: the name cut in the middle, its extension kept.
#[must_use]
pub fn one_line(text: &str, has_ext: bool, width: f32, measure: &dyn Fn(&str) -> f32) -> Caption {
    caption(text, has_ext, 1, width, measure)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every character 6 pt wide.
    fn mono(s: &str) -> f32 {
        s.chars().count() as f32 * 6.0
    }

    #[test]
    fn short_names_stay_whole() {
        let c = caption("box.ogf", true, 2, 100.0, &mono);
        assert_eq!(c.lines, ["box.ogf"]);
        assert!(!c.truncated);
    }

    #[test]
    fn breaks_where_a_reader_does() {
        // 10 characters a line
        let c = caption("bytil_wnd.dds", true, 2, 60.0, &mono);
        assert_eq!(c.lines, ["bytil_", "wnd.dds"]);
        let c = caption("NewBehaviour", false, 2, 60.0, &mono);
        assert_eq!(c.lines, ["New", "Behaviour"]);
    }

    #[test]
    fn never_right_before_the_extension() {
        // "abcdefghij" fills a line: the cut goes one character back, so ".dds" is not alone
        let c = caption("abcdefghijk.dds", true, 2, 66.0, &mono);
        assert_eq!(c.lines.first().map(String::as_str), Some("abcdefghij"));
        assert!(c.lines.get(1).is_some_and(|l| l.starts_with('k')));
    }

    #[test]
    fn the_last_line_keeps_the_extension() {
        let c = caption("a_very_long_texture_name_here.dds", true, 2, 60.0, &mono);
        assert!(c.truncated);
        let last = c.lines.last().cloned().unwrap_or_default();
        assert!(last.ends_with(".dds") && last.contains(ELLIPSIS), "{last}");
        assert!(mono(&last) <= 60.0);
    }

    #[test]
    fn one_line_cuts_too() {
        let c = one_line("ночная_прогулка.nqasset", false, 48.0, &mono);
        assert!(c.truncated && c.lines.len() == 1);
    }
}
